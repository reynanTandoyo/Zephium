//! An in-process IMAP server over `tokio::io::duplex`, speaking just enough
//! of RFC 3501 for the engine's flows. No sockets, no real network.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::Notify;
use zephium_mail::error::Result;
use zephium_mail::model::Endpoint;
use zephium_mail::transport::{BoxStream, Connector};

pub const PASSWORD: &str = "secret";
/// The bearer token the fake accepts for `AUTHENTICATE XOAUTH2`.
pub const TOKEN: &str = "ya29.fake-token";

#[derive(Clone, Debug)]
pub struct Msg {
    pub uid: u32,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct FolderData {
    pub name: &'static str,
    pub wire: &'static str,
    pub attrs: &'static str,
    pub msgs: Vec<Msg>,
    pub next_uid: u32,
}

impl FolderData {
    pub fn new(name: &'static str, wire: &'static str, attrs: &'static str) -> Self {
        Self {
            name,
            wire,
            attrs,
            msgs: Vec::new(),
            next_uid: 1,
        }
    }
    pub fn add(&mut self, flags: &[&str], raw: &[u8]) -> u32 {
        let uid = self.next_uid;
        self.next_uid += 1;
        self.msgs.push(Msg {
            uid,
            flags: flags.iter().map(|f| (*f).to_owned()).collect(),
            raw: raw.to_vec(),
        });
        uid
    }
}

/// How the fake misbehaves on purpose.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Hostile {
    #[default]
    None,
    /// `LIST` never ends: an endless run of folder lines.
    EndlessList,
    /// `LIST` sends one line that never ends.
    EndlessLine,
    /// `UID FETCH` of headers never ends: an endless run of rows.
    EndlessRows,
    /// `RFC822.SIZE` says 100 bytes, then `BODY[]` is a literal of 64 MiB.
    LyingSize,
}

#[derive(Default)]
pub struct State {
    pub hostile: Hostile,
    pub folders: Vec<FolderData>,
    pub log: Vec<String>,
    /// A command verb the server never answers.
    pub silent_on: Option<&'static str>,
    /// Answer SELECT/EXAMINE with a hostile NO.
    pub refuse_select: bool,
}

pub fn simple(from: &str, subject: &str, body: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: me@example.com\r\nSubject: {subject}\r\nDate: Wed, 13 Nov 2024 12:00:00 +0000\r\nMessage-ID: <{subject}@fake>\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

pub fn standard_folders() -> Vec<FolderData> {
    vec![
        FolderData::new("INBOX", "INBOX", "\\HasNoChildren"),
        FolderData::new("Sent Items", "Sent Items", "\\HasNoChildren \\Sent"),
        FolderData::new("Entwürfe", "Entw&APw-rfe", "\\HasNoChildren"),
        FolderData::new("Trash", "Trash", "\\HasNoChildren \\Trash"),
        FolderData::new("Archive", "Archive", "\\HasNoChildren \\Archive"),
        FolderData::new("Projects", "Projects", "\\Noselect \\HasChildren"),
    ]
}

pub struct FakeConnector {
    pub state: Arc<Mutex<State>>,
    pub idle: Arc<Notify>,
    pub connects: AtomicUsize,
    /// Bumping this makes every live server task hang up.
    pub generation: Arc<AtomicU64>,
}

impl FakeConnector {
    pub fn new(folders: Vec<FolderData>) -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(State {
                folders,
                ..State::default()
            })),
            idle: Arc::new(Notify::new()),
            connects: AtomicUsize::new(0),
            generation: Arc::new(AtomicU64::new(0)),
        })
    }
    pub fn hang_up_all(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
    pub fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().log.clone()
    }
}

impl Connector for FakeConnector {
    fn connect<'a>(&'a self, _endpoint: &'a Endpoint) -> BoxFuture<'a, Result<BoxStream>> {
        Box::pin(async move {
            self.connects.fetch_add(1, Ordering::SeqCst);
            let (client, server) = tokio::io::duplex(1 << 20);
            let generation = self.generation.load(Ordering::SeqCst);
            tokio::spawn(serve(
                server,
                self.state.clone(),
                self.idle.clone(),
                self.generation.clone(),
                generation,
            ));
            Ok(Box::new(client) as BoxStream)
        })
    }
}

fn hostile_now(state: &Mutex<State>) -> Hostile {
    state.lock().unwrap().hostile
}

fn uids_of(set: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in set.split(',') {
        if let Some((a, b)) = part.split_once(':') {
            let (a, b): (u32, u32) = (a.parse().unwrap_or(1), b.parse().unwrap_or(u32::MAX));
            out.extend(a..=b.min(a + 10_000));
        } else if let Ok(n) = part.parse() {
            out.push(n);
        }
    }
    out
}

fn unquote(text: &str) -> String {
    text.trim()
        .trim_matches('"')
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
}

fn find_folder<'a>(state: &'a mut State, wire: &str) -> Option<&'a mut FolderData> {
    let wire = unquote(wire);
    state.folders.iter_mut().find(|f| f.wire == wire)
}

fn split_header(raw: &[u8]) -> (&[u8], &[u8]) {
    let at = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(raw.len(), |p| p + 4);
    raw.split_at(at)
}

async fn serve(
    io: DuplexStream,
    state: Arc<Mutex<State>>,
    idle: Arc<Notify>,
    generation: Arc<AtomicU64>,
    born: u64,
) {
    let (reader, mut w) = tokio::io::split(io);
    let mut r = BufReader::new(reader);
    let _ = w.write_all(b"* OK fake ready\r\n").await;
    let mut selected: Option<String> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        if generation.load(Ordering::SeqCst) != born {
            return;
        }
        let line = line.trim_end().to_owned();
        let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
        let verb = rest.split(' ').next().unwrap_or("").to_ascii_uppercase();
        let logged = if verb == "LOGIN" || verb == "AUTHENTICATE" {
            format!("{verb} <redacted>")
        } else {
            rest.to_owned()
        };
        let silent = {
            let mut s = state.lock().unwrap();
            s.log.push(logged);
            s.silent_on.is_some_and(|v| v == verb)
        };
        if silent {
            continue;
        }
        let mut out: Vec<u8> = Vec::new();
        let ok = |out: &mut Vec<u8>, text: &str| {
            out.extend_from_slice(format!("{tag} OK {text}\r\n").as_bytes())
        };
        match verb.as_str() {
            "CAPABILITY" => {
                out.extend_from_slice(
                    b"* CAPABILITY IMAP4rev1 IDLE MOVE UIDPLUS CONDSTORE SPECIAL-USE\r\n",
                );
                ok(&mut out, "done");
            }
            "LOGIN" => {
                let pass = rest.split(' ').nth(2).map(unquote).unwrap_or_default();
                if pass == PASSWORD {
                    ok(&mut out, "logged in");
                } else {
                    out.extend_from_slice(
                        format!("{tag} NO [AUTHENTICATIONFAILED] SERVER-SECRET-TEXT\r\n")
                            .as_bytes(),
                    );
                }
            }
            "AUTHENTICATE" => {
                // SASL: empty continuation, then one base64 line.
                let _ = w.write_all(b"+ \r\n").await;
                let mut answer = String::new();
                if r.read_line(&mut answer).await.unwrap_or(0) == 0 {
                    return;
                }
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(answer.trim())
                    .unwrap_or_default();
                let expected = format!("user=me\u{1}auth=Bearer {TOKEN}\u{1}\u{1}");
                if rest.to_ascii_uppercase().ends_with("XOAUTH2") && decoded == expected.as_bytes()
                {
                    ok(&mut out, "authenticated");
                } else {
                    out.extend_from_slice(
                        format!("{tag} NO [AUTHENTICATIONFAILED] SERVER-SECRET-TEXT\r\n")
                            .as_bytes(),
                    );
                }
            }
            "NOOP" => ok(&mut out, "done"),
            "LOGOUT" => {
                out.extend_from_slice(b"* BYE bye\r\n");
                ok(&mut out, "done");
                let _ = w.write_all(&out).await;
                return;
            }
            "LIST" if hostile_now(&state) == Hostile::EndlessList => {
                let mut n = 0u64;
                loop {
                    let line = format!("* LIST (\\HasNoChildren) \"/\" \"F{n}\"\r\n");
                    if w.write_all(line.as_bytes()).await.is_err() {
                        return;
                    }
                    n += 1;
                }
            }
            "LIST" if hostile_now(&state) == Hostile::EndlessLine => {
                if w.write_all(b"* LIST (").await.is_err() {
                    return;
                }
                let chunk = [b'x'; 8192];
                while w.write_all(&chunk).await.is_ok() {}
                return;
            }
            "LIST" => {
                let s = state.lock().unwrap();
                for f in &s.folders {
                    out.extend_from_slice(
                        format!("* LIST ({}) \"/\" \"{}\"\r\n", f.attrs, f.wire).as_bytes(),
                    );
                }
                ok(&mut out, "done");
            }
            "STATUS" => {
                let name = rest.split(' ').nth(1).unwrap_or("");
                let mut s = state.lock().unwrap();
                match find_folder(&mut s, name) {
                    Some(f) => {
                        let unseen = f
                            .msgs
                            .iter()
                            .filter(|m| !m.flags.iter().any(|x| x == "\\Seen"))
                            .count();
                        out.extend_from_slice(
                            format!(
                                "* STATUS \"{}\" (MESSAGES {} UNSEEN {unseen} UIDNEXT {} UIDVALIDITY 7 HIGHESTMODSEQ 42)\r\n",
                                f.wire, f.msgs.len(), f.next_uid
                            )
                            .as_bytes(),
                        );
                        ok(&mut out, "done");
                    }
                    None => out.extend_from_slice(format!("{tag} NO nope\r\n").as_bytes()),
                }
            }
            "SELECT" | "EXAMINE" => {
                let name = rest.split(' ').nth(1).unwrap_or("");
                let mut s = state.lock().unwrap();
                if s.refuse_select {
                    out.extend_from_slice(
                        format!("{tag} NO SERVER-SECRET-TEXT mailbox on fire\r\n").as_bytes(),
                    );
                } else if let Some(f) = find_folder(&mut s, name) {
                    selected = Some(f.wire.to_owned());
                    out.extend_from_slice(
                        format!(
                            "* {} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 7] ok\r\n* OK [UIDNEXT {}] ok\r\n* FLAGS (\\Seen \\Flagged \\Deleted \\Draft \\Answered)\r\n{tag} OK [READ-WRITE] done\r\n",
                            f.msgs.len(), f.next_uid
                        )
                        .as_bytes(),
                    );
                } else {
                    out.extend_from_slice(format!("{tag} NO no such mailbox\r\n").as_bytes());
                }
            }
            "UID"
                if rest.to_ascii_uppercase().contains("FETCH") && {
                    let hostile = hostile_now(&state);
                    (hostile == Hostile::EndlessRows && rest.contains("BODY.PEEK[HEADER]"))
                        || (hostile == Hostile::LyingSize && rest.contains("BODY.PEEK[]"))
                } =>
            {
                if hostile_now(&state) == Hostile::EndlessRows {
                    let mut n = 0u64;
                    loop {
                        n += 1;
                        let row = format!("* {n} FETCH (UID {n} FLAGS (\\Seen))\r\n");
                        if w.write_all(row.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
                // LyingSize: a literal of 64 MiB for a message "of 100 bytes".
                let total = 64usize << 20;
                let head = format!("* 1 FETCH (UID 1 BODY[] {{{total}}}\r\n");
                if w.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                let chunk = [b'a'; 65536];
                for _ in 0..(total / chunk.len()) {
                    if w.write_all(&chunk).await.is_err() {
                        return;
                    }
                }
                let _ = w.write_all(b")\r\n").await;
                continue;
            }
            "UID" => {
                let mut parts = rest.splitn(4, ' ');
                parts.next();
                let sub = parts.next().unwrap_or("").to_ascii_uppercase();
                let set = parts.next().unwrap_or("");
                let args = parts.next().unwrap_or("");
                let mut s = state.lock().unwrap();
                let wire = selected.clone().unwrap_or_default();
                match sub.as_str() {
                    "FETCH" => {
                        let uids = uids_of(set);
                        let f = find_folder(&mut s, &wire).expect("selected");
                        for (i, m) in f
                            .msgs
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| uids.contains(&m.uid))
                        {
                            let flags = m.flags.join(" ");
                            if args.contains("BODY.PEEK[HEADER]") {
                                let (head, body) = split_header(&m.raw);
                                let body = &body[..body.len().min(1024)];
                                out.extend_from_slice(format!("* {} FETCH (UID {} FLAGS ({flags}) RFC822.SIZE {} INTERNALDATE \"13-Nov-2024 12:00:00 +0000\" BODY[HEADER] {{{}}}\r\n", i + 1, m.uid, m.raw.len(), head.len()).as_bytes());
                                out.extend_from_slice(head);
                                out.extend_from_slice(
                                    format!(" BODY[TEXT]<0> {{{}}}\r\n", body.len()).as_bytes(),
                                );
                                out.extend_from_slice(body);
                                out.extend_from_slice(b")\r\n");
                            } else if args.contains("BODY.PEEK[]") {
                                out.extend_from_slice(
                                    format!(
                                        "* {} FETCH (UID {} BODY[] {{{}}}\r\n",
                                        i + 1,
                                        m.uid,
                                        m.raw.len()
                                    )
                                    .as_bytes(),
                                );
                                out.extend_from_slice(&m.raw);
                                out.extend_from_slice(b")\r\n");
                            } else {
                                out.extend_from_slice(
                                    format!(
                                        "* {} FETCH (UID {} RFC822.SIZE {})\r\n",
                                        i + 1,
                                        m.uid,
                                        m.raw.len()
                                    )
                                    .as_bytes(),
                                );
                            }
                        }
                        ok(&mut out, "done");
                    }
                    "STORE" => {
                        let uids = uids_of(set);
                        let add = args.starts_with('+');
                        let flag = args
                            .split('(')
                            .nth(1)
                            .unwrap_or("")
                            .trim_end_matches(')')
                            .to_owned();
                        let f = find_folder(&mut s, &wire).expect("selected");
                        for m in f.msgs.iter_mut().filter(|m| uids.contains(&m.uid)) {
                            m.flags.retain(|x| *x != flag);
                            if add {
                                m.flags.push(flag.clone());
                            }
                        }
                        ok(&mut out, "done");
                    }
                    "MOVE" | "COPY" => {
                        let uids = uids_of(set);
                        let dest = unquote(args);
                        let moved: Vec<Msg> = {
                            let f = find_folder(&mut s, &wire).expect("selected");
                            let taken: Vec<Msg> = f
                                .msgs
                                .iter()
                                .filter(|m| uids.contains(&m.uid))
                                .cloned()
                                .collect();
                            if sub == "MOVE" {
                                f.msgs.retain(|m| !uids.contains(&m.uid));
                            }
                            taken
                        };
                        match find_folder(&mut s, &dest) {
                            Some(d) => {
                                for m in moved {
                                    d.add(
                                        &m.flags.iter().map(String::as_str).collect::<Vec<_>>(),
                                        &m.raw,
                                    );
                                }
                                ok(&mut out, "done");
                            }
                            None => out.extend_from_slice(
                                format!("{tag} NO [TRYCREATE] no dest\r\n").as_bytes(),
                            ),
                        }
                    }
                    "EXPUNGE" => {
                        let uids = uids_of(set);
                        let f = find_folder(&mut s, &wire).expect("selected");
                        f.msgs.retain(|m| {
                            !(uids.contains(&m.uid) && m.flags.iter().any(|x| x == "\\Deleted"))
                        });
                        ok(&mut out, "done");
                    }
                    "SEARCH" => {
                        let needle = {
                            let all = format!("{set} {args}");
                            let start = all.find('"').map_or(0, |p| p + 1);
                            let end = all[start..].find('"').map_or(all.len(), |p| p + start);
                            all[start..end].to_lowercase()
                        };
                        let f = find_folder(&mut s, &wire).expect("selected");
                        let hits: Vec<String> = f
                            .msgs
                            .iter()
                            .filter(|m| {
                                String::from_utf8_lossy(&m.raw)
                                    .to_lowercase()
                                    .contains(&needle)
                            })
                            .map(|m| m.uid.to_string())
                            .collect();
                        out.extend_from_slice(
                            format!("* SEARCH {}\r\n", hits.join(" ")).as_bytes(),
                        );
                        ok(&mut out, "done");
                    }
                    _ => out.extend_from_slice(format!("{tag} BAD unknown\r\n").as_bytes()),
                }
            }
            "APPEND" => {
                // APPEND "Folder" [(flags)] {n}
                let size: usize = rest
                    .rsplit('{')
                    .next()
                    .unwrap_or("0")
                    .trim_end_matches('}')
                    .parse()
                    .unwrap_or(0);
                let name = rest
                    .split_once(' ')
                    .map_or("", |(_, tail)| tail)
                    .rsplit_once('"')
                    .map_or(String::new(), |(head, _)| format!("{head}\""));
                let flags: Vec<String> = rest
                    .split('(')
                    .nth(1)
                    .and_then(|x| x.split(')').next())
                    .map(|x| {
                        x.split(' ')
                            .filter(|f| !f.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = w.write_all(b"+ go ahead\r\n").await;
                let mut raw = vec![0u8; size];
                if r.read_exact(&mut raw).await.is_err() {
                    return;
                }
                let mut crlf = String::new();
                let _ = r.read_line(&mut crlf).await;
                let mut s = state.lock().unwrap();
                match find_folder(&mut s, &name) {
                    Some(f) => {
                        f.add(&flags.iter().map(String::as_str).collect::<Vec<_>>(), &raw);
                        ok(&mut out, "appended");
                    }
                    None => out.extend_from_slice(
                        format!("{tag} NO [TRYCREATE] no such folder\r\n").as_bytes(),
                    ),
                }
            }
            "IDLE" => {
                let _ = w.write_all(b"+ idling\r\n").await;
                let mut done = String::new();
                tokio::select! {
                    _ = r.read_line(&mut done) => {}
                    () = idle.notified() => {
                        let _ = w.write_all(b"* 3 EXISTS\r\n").await;
                        let _ = r.read_line(&mut done).await;
                    }
                }
                ok(&mut out, "idle terminated");
            }
            _ => out.extend_from_slice(format!("{tag} BAD unknown command\r\n").as_bytes()),
        }
        if w.write_all(&out).await.is_err() {
            return;
        }
    }
}
