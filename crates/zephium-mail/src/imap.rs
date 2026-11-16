//! One authenticated IMAP session (T-0020, T-0032): folders with SPECIAL-USE
//! roles, header pages by UID range, bodies on demand, flags, move/copy,
//! `UID SEARCH` and IDLE. Every command runs under [`COMMAND_TIMEOUT`] and
//! every failure maps to [`MailError`] without server text.

use std::collections::HashSet;
use std::sync::Arc;

use async_imap::imap_proto::types::NameAttribute;
use async_imap::types::Flag;
use async_imap::{Client, Session};
use futures::TryStreamExt;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::auth::{Credential, XOauth2};
use crate::error::{MailError, Result};
use crate::limits::{
    COMMAND_TIMEOUT, CONNECT_TIMEOUT, FETCH_DEADLINE, IDLE_RENEW, LIST_DEADLINE,
    MAX_FETCH_OVERHEAD_BYTES, MAX_FOLDERS, MAX_FOLDER_NAME_BYTES, MAX_HEADERS_REPLY_BYTES,
    MAX_LIST_ANSWERS, MAX_MESSAGE_BYTES, MAX_PAGE, MAX_QUERY_BYTES, MAX_REPLY_BYTES,
    MAX_SEARCH_HITS, MAX_SEARCH_REPLY_BYTES,
};
use crate::metered::{Metered, ReadBudget};
use crate::mime;
use crate::model::{
    Endpoint, FlagKind, Flags, Folder, FolderRole, HeaderPage, MessageBody, MessageHeader,
    Security, WatchOutcome,
};
use crate::transport::{BoxStream, Connector};
use crate::utf7;

/// The most UIDs one command names.
pub const MAX_UIDS_PER_COMMAND: usize = 1000;

/// What the server advertised.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    /// `IDLE`.
    pub idle: bool,
    /// `MOVE`.
    pub move_ext: bool,
    /// `UIDPLUS` (needed for a safe `UID EXPUNGE`).
    pub uidplus: bool,
    /// `CONDSTORE`.
    pub condstore: bool,
}

/// An authenticated session on one connection.
pub struct ImapSession {
    session: Session<BoxStream>,
    caps: Capabilities,
    selected: Option<(String, bool)>,
    /// What this connection's server may still send for the current command.
    budget: Arc<ReadBudget>,
}

impl std::fmt::Debug for ImapSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImapSession")
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

async fn bounded<T>(
    future: impl std::future::Future<Output = async_imap::error::Result<T>>,
) -> Result<T> {
    Ok(timeout(COMMAND_TIMEOUT, future).await??)
}

/// A comma list of UIDs, or `InvalidInput` for none, zero or too many.
pub fn uid_set(uids: &[u32]) -> Result<String> {
    if uids.is_empty() || uids.len() > MAX_UIDS_PER_COMMAND || uids.contains(&0) {
        return Err(MailError::InvalidInput);
    }
    Ok(uids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(","))
}

fn folder_name_ok(path: &str) -> bool {
    !path.is_empty() && path.len() <= MAX_FOLDER_NAME_BYTES && !path.chars().any(char::is_control)
}

fn wire_name(path: &str) -> Result<String> {
    if !folder_name_ok(path) {
        return Err(MailError::InvalidInput);
    }
    Ok(utf7::encode(path))
}

/// Quotes a search string for the wire. Non-ASCII text relies on
/// `CHARSET UTF-8`, which the caller adds.
fn quote_search(text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_QUERY_BYTES
        || trimmed.chars().any(char::is_control)
    {
        return Err(MailError::InvalidInput);
    }
    let mut out = String::with_capacity(trimmed.len() + 2);
    out.push('"');
    for ch in trimmed.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    Ok(out)
}

/// The `UID SEARCH` arguments for a text query over subject, sender and body.
pub fn search_command(query: &str) -> Result<String> {
    let quoted = quote_search(query)?;
    let charset = if quoted.is_ascii() {
        ""
    } else {
        "CHARSET UTF-8 "
    };
    Ok(format!(
        "{charset}OR SUBJECT {quoted} OR FROM {quoted} TEXT {quoted}"
    ))
}

// Folder names servers without SPECIAL-USE use for each role, lower case, in
// the 13 languages of the app (en, de, es, fr, it, ja, ko, pt-BR, ru, tr, uk,
// zh-CN, zh-TW). The last path segment is compared.
const SENT_NAMES: &[&str] = &[
    "sent",
    "sent items",
    "sent mail",
    "sent messages",
    "gesendet",
    "gesendete elemente",
    "gesendete objekte",
    "enviados",
    "enviado",
    "elementos enviados",
    "mensajes enviados",
    "envoyés",
    "envoyes",
    "éléments envoyés",
    "messages envoyés",
    "posta inviata",
    "inviati",
    "inviata",
    "elementi inviati",
    "送信済み",
    "送信済みアイテム",
    "送信済みメッセージ",
    "보낸 편지함",
    "보낸편지함",
    "보낸 메일함",
    "itens enviados",
    "mensagens enviadas",
    "enviadas",
    "отправленные",
    "отправленные элементы",
    "gönderilmiş öğeler",
    "gönderilen öğeler",
    "gönderilenler",
    "gönderilmiş",
    "надіслані",
    "відправлені",
    "已发送",
    "已发送邮件",
    "寄件備份",
    "已傳送",
    "已傳送郵件",
];
const DRAFT_NAMES: &[&str] = &[
    "drafts",
    "draft",
    "entwürfe",
    "entwurf",
    "borradores",
    "borrador",
    "brouillons",
    "brouillon",
    "bozze",
    "bozza",
    "下書き",
    "임시 보관함",
    "임시보관함",
    "임시 저장",
    "rascunhos",
    "rascunho",
    "черновики",
    "taslaklar",
    "taslak",
    "чернетки",
    "草稿",
    "草稿箱",
    "草稿匣",
];
const TRASH_NAMES: &[&str] = &[
    "trash",
    "deleted items",
    "deleted messages",
    "bin",
    "papierkorb",
    "gelöschte elemente",
    "gelöschte objekte",
    "gelöscht",
    "papelera",
    "elementos eliminados",
    "eliminados",
    "mensajes eliminados",
    "corbeille",
    "éléments supprimés",
    "supprimés",
    "messages supprimés",
    "cestino",
    "posta eliminata",
    "elementi eliminati",
    "ごみ箱",
    "削除済みアイテム",
    "削除済み",
    "휴지통",
    "지운 편지함",
    "삭제된 항목",
    "lixeira",
    "itens excluídos",
    "itens eliminados",
    "excluídos",
    "lixo",
    "корзина",
    "удалённые",
    "удаленные",
    "удалённые элементы",
    "çöp kutusu",
    "silinmiş öğeler",
    "silinen öğeler",
    "çöp",
    "кошик",
    "видалені",
    "已删除",
    "已删除邮件",
    "已删除项目",
    "废纸篓",
    "回收站",
    "垃圾桶",
    "已刪除",
    "已刪除郵件",
    "刪除的郵件",
    "資源回收筒",
];
const ARCHIVE_NAMES: &[&str] = &[
    "archive",
    "archives",
    "archiv",
    "archivo",
    "archivados",
    "archivés",
    "archivio",
    "アーカイブ",
    "보관함",
    "보관 메일함",
    "arquivo",
    "arquivados",
    "arquivo morto",
    "архив",
    "arşiv",
    "архів",
    "归档",
    "存档",
    "封存",
    "封存郵件",
    "歸檔",
];
const JUNK_NAMES: &[&str] = &[
    "junk",
    "spam",
    "junk e-mail",
    "junk-e-mail",
    "bulk mail",
    "correo no deseado",
    "no deseado",
    "courrier indésirable",
    "indésirables",
    "pourriel",
    "posta indesiderata",
    "indesiderata",
    "迷惑メール",
    "スパム",
    "스팸",
    "스팸 메일함",
    "정크 메일",
    "lixo eletrônico",
    "lixo eletronico",
    "нежелательная почта",
    "спам",
    "gereksiz",
    "gereksiz e-posta",
    "önemsiz",
    "небажана пошта",
    "垃圾邮件",
    "垃圾箱",
    "垃圾郵件",
    "垃圾信件",
];

fn role_from_name(path: &str) -> FolderRole {
    let last = path
        .rsplit(['/', '.', '\\'])
        .next()
        .unwrap_or(path)
        .trim()
        .to_lowercase();
    let last = last.as_str();
    if last == "inbox" {
        FolderRole::Inbox
    } else if SENT_NAMES.contains(&last) {
        FolderRole::Sent
    } else if DRAFT_NAMES.contains(&last) {
        FolderRole::Drafts
    } else if TRASH_NAMES.contains(&last) {
        FolderRole::Trash
    } else if ARCHIVE_NAMES.contains(&last) {
        FolderRole::Archive
    } else if JUNK_NAMES.contains(&last) {
        FolderRole::Junk
    } else {
        FolderRole::Other
    }
}

fn role_from_attributes(attributes: &[NameAttribute<'_>]) -> Option<FolderRole> {
    attributes.iter().find_map(|attribute| match attribute {
        NameAttribute::Sent => Some(FolderRole::Sent),
        NameAttribute::Drafts => Some(FolderRole::Drafts),
        NameAttribute::Trash => Some(FolderRole::Trash),
        NameAttribute::Archive => Some(FolderRole::Archive),
        NameAttribute::Junk => Some(FolderRole::Junk),
        _ => None,
    })
}

fn flags_of<'a>(flags: impl Iterator<Item = Flag<'a>>) -> Flags {
    let mut out = Flags::default();
    for flag in flags {
        match flag {
            Flag::Seen => out.seen = true,
            Flag::Flagged => out.flagged = true,
            Flag::Answered => out.answered = true,
            Flag::Draft => out.draft = true,
            Flag::Deleted => out.deleted = true,
            _ => {}
        }
    }
    out
}

impl ImapSession {
    /// Connects, reads the greeting and authenticates: `LOGIN` for a
    /// password, `AUTHENTICATE XOAUTH2` for a bearer token.
    pub async fn connect(
        connector: &dyn Connector,
        endpoint: &Endpoint,
        username: &str,
        credential: &Credential,
    ) -> Result<Self> {
        let stream = timeout(CONNECT_TIMEOUT * 2, connector.connect(endpoint)).await??;
        // Every byte the server sends is counted; see `metered`.
        let budget = ReadBudget::new(MAX_REPLY_BYTES);
        let stream: BoxStream = Box::new(Metered::new(stream, budget.clone()));
        let mut client = Client::new(stream);
        if endpoint.security == Security::Tls {
            // STARTTLS connectors consumed the greeting before upgrading.
            timeout(CONNECT_TIMEOUT, client.read_response())
                .await?
                .map_err(|_| MailError::Offline)?
                .ok_or(MailError::Offline)?;
        }
        let attempt = match credential {
            Credential::Password(password) => {
                timeout(COMMAND_TIMEOUT, client.login(username, password.as_str())).await?
            }
            Credential::Bearer(token) => {
                let responder = XOauth2::new(username, token)?;
                timeout(COMMAND_TIMEOUT, client.authenticate("XOAUTH2", responder)).await?
            }
        };
        let session = match attempt {
            Ok(session) => session,
            Err((async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_), _)) => {
                return Err(MailError::AuthFailed)
            }
            Err((error, _)) => return Err(error.into()),
        };
        let mut session = Self {
            session,
            caps: Capabilities::default(),
            selected: None,
            budget,
        };
        session.arm(MAX_REPLY_BYTES);
        let caps = bounded(session.session.capabilities()).await?;
        session.caps = Capabilities {
            idle: caps.has_str("IDLE"),
            move_ext: caps.has_str("MOVE"),
            uidplus: caps.has_str("UIDPLUS"),
            condstore: caps.has_str("CONDSTORE"),
        };
        Ok(session)
    }

    /// What the server advertised at login.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// Starts a command: the server may send `limit` more bytes for it.
    fn arm(&self, limit: u64) {
        self.budget.arm(limit);
    }

    /// True once the server sent more than a command allows. The stream is
    /// then out of step with the client, so the connection must be dropped.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.budget.tripped()
    }

    fn drain_unsolicited(&mut self) {
        while self.session.unsolicited_responses.try_recv().is_ok() {}
    }

    /// Keeps the connection alive and surfaces a dropped one.
    pub async fn noop(&mut self) -> Result<()> {
        self.arm(MAX_REPLY_BYTES);
        let result = bounded(self.session.noop()).await;
        self.drain_unsolicited();
        result
    }

    /// Logs out, ignoring failures.
    pub async fn logout(mut self) {
        let _ = timeout(COMMAND_TIMEOUT, self.session.logout()).await;
    }

    async fn select(&mut self, path: &str, write: bool) -> Result<async_imap::types::Mailbox> {
        let wire = wire_name(path)?;
        self.arm(MAX_REPLY_BYTES);
        let result = if write {
            bounded(self.session.select(&wire)).await
        } else {
            bounded(self.session.examine(&wire)).await
        };
        self.drain_unsolicited();
        match result {
            // A server must report UIDVALIDITY on SELECT. Its absence means the
            // stream ended mid-command, which must read as a dead connection and
            // never as an empty folder.
            Ok(mailbox) if mailbox.uid_validity.is_none() => {
                self.selected = None;
                Err(MailError::Offline)
            }
            Ok(mailbox) => {
                self.selected = Some((path.to_owned(), write));
                Ok(mailbox)
            }
            Err(error) => {
                self.selected = None;
                Err(if error == MailError::ServerRejected {
                    MailError::NotFound
                } else {
                    error
                })
            }
        }
    }

    /// Lists folders with roles; `with_status` adds unread and totals.
    pub async fn list_folders(&mut self, with_status: bool) -> Result<Vec<Folder>> {
        let mut folders: Vec<Folder> = Vec::new();
        let mut seen = HashSet::new();
        self.arm(MAX_REPLY_BYTES);
        let listing = async {
            let names = timeout(COMMAND_TIMEOUT, self.session.list(Some(""), Some("*")))
                .await?
                .map_err(MailError::from)?;
            futures::pin_mut!(names);
            let mut answers = 0usize;
            while let Some(name) = timeout(COMMAND_TIMEOUT, names.try_next())
                .await?
                .map_err(MailError::from)?
            {
                // Past the cap the rest is read and dropped, but only so far:
                // a list that never ends is a broken or hostile server.
                answers += 1;
                if answers > MAX_LIST_ANSWERS {
                    return Err(MailError::Protocol);
                }
                if folders.len() >= MAX_FOLDERS {
                    continue;
                }
                let Some(path) = utf7::decode(name.name()) else {
                    continue;
                };
                if !folder_name_ok(&path) || !seen.insert(path.clone()) {
                    continue;
                }
                let delimiter = name.delimiter().and_then(|d| d.chars().next());
                let display = match delimiter {
                    Some(d) => path.rsplit(d).next().unwrap_or(&path).to_owned(),
                    None => path.clone(),
                };
                let role = if path.eq_ignore_ascii_case("INBOX") {
                    FolderRole::Inbox
                } else {
                    role_from_attributes(name.attributes()).unwrap_or_else(|| role_from_name(&path))
                };
                let selectable = !name
                    .attributes()
                    .iter()
                    .any(|attribute| matches!(attribute, NameAttribute::NoSelect));
                folders.push(Folder {
                    path,
                    name: display,
                    delimiter,
                    role,
                    unread: 0,
                    total: 0,
                    uid_validity: None,
                    uid_next: None,
                    highest_modseq: None,
                    selectable,
                });
            }
            Ok::<(), MailError>(())
        };
        timeout(LIST_DEADLINE, listing).await??;
        self.drain_unsolicited();
        // A second folder claiming a unique role loses it, so each role has one owner.
        let mut claimed = HashSet::new();
        for folder in &mut folders {
            if folder.role != FolderRole::Other && !claimed.insert(folder.role) {
                folder.role = FolderRole::Other;
            }
        }
        if with_status {
            let items = if self.caps.condstore {
                "(MESSAGES UNSEEN UIDNEXT UIDVALIDITY HIGHESTMODSEQ)"
            } else {
                "(MESSAGES UNSEEN UIDNEXT UIDVALIDITY)"
            };
            for folder in folders.iter_mut().filter(|folder| folder.selectable) {
                let wire = wire_name(&folder.path)?;
                self.arm(MAX_REPLY_BYTES);
                if let Ok(status) = bounded(self.session.status(&wire, items)).await {
                    folder.unread = status.unseen.unwrap_or(0);
                    folder.total = status.exists;
                    folder.uid_validity = status.uid_validity;
                    folder.uid_next = status.uid_next;
                    folder.highest_modseq = status.highest_modseq;
                }
            }
            self.drain_unsolicited();
        }
        Ok(folders)
    }

    async fn fetch_headers(
        &mut self,
        set: &str,
        low: u32,
        high: u32,
    ) -> Result<Vec<MessageHeader>> {
        let query =
            "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.1024>)";
        let mut rows: Vec<MessageHeader> = Vec::new();
        self.arm(MAX_HEADERS_REPLY_BYTES);
        let fetching = async {
            let stream = timeout(COMMAND_TIMEOUT, self.session.uid_fetch(set, query))
                .await?
                .map_err(MailError::from)?;
            futures::pin_mut!(stream);
            let mut answers = 0usize;
            while let Some(fetch) = timeout(COMMAND_TIMEOUT, stream.try_next())
                .await?
                .map_err(MailError::from)?
            {
                // The range names at most one page; far more rows than that
                // (or an unbounded number of them) means a hostile server.
                answers += 1;
                if answers > MAX_PAGE as usize * 4 {
                    return Err(MailError::Protocol);
                }
                let Some(uid) = fetch.uid else { continue };
                if uid < low || uid > high || rows.len() >= MAX_PAGE as usize * 2 {
                    continue;
                }
                let Some(header) = fetch.header() else {
                    continue;
                };
                let Ok(mut parsed) = mime::parse_list_header(header, fetch.text()) else {
                    continue;
                };
                parsed.uid = uid;
                parsed.flags = flags_of(fetch.flags());
                parsed.size = fetch.size.unwrap_or(0);
                if parsed.date.is_none() {
                    parsed.date = fetch.internal_date().map(|date| date.timestamp());
                }
                rows.push(parsed);
            }
            Ok::<(), MailError>(())
        };
        timeout(LIST_DEADLINE, fetching).await??;
        self.drain_unsolicited();
        rows.sort_by_key(|row| std::cmp::Reverse(row.uid));
        rows.dedup_by_key(|row| row.uid);
        Ok(rows)
    }

    /// One page of headers, newest first. `before_uid` is exclusive; pass the
    /// previous page's `next_before` to continue.
    pub async fn headers(
        &mut self,
        folder: &str,
        before_uid: Option<u32>,
        limit: u32,
    ) -> Result<HeaderPage> {
        let mailbox = self.select(folder, false).await?;
        let limit = limit.clamp(1, MAX_PAGE);
        let ceiling = mailbox
            .uid_next
            .map_or(u32::MAX, |next| next.saturating_sub(1));
        let top = before_uid.map_or(ceiling, |before| before.saturating_sub(1).min(ceiling));
        if mailbox.exists == 0 || top == 0 {
            return Ok(HeaderPage {
                messages: Vec::new(),
                next_before: None,
                uid_validity: mailbox.uid_validity,
                total: mailbox.exists,
            });
        }
        let low = top.saturating_sub(limit - 1).max(1);
        let messages = self
            .fetch_headers(&format!("{low}:{top}"), low, top)
            .await?;
        Ok(HeaderPage {
            messages,
            next_before: (low > 1).then_some(low),
            uid_validity: mailbox.uid_validity,
            total: mailbox.exists,
        })
    }

    /// The raw bytes of one message. Refuses before downloading when the
    /// server reports a size over [`MAX_MESSAGE_BYTES`], and does not trust
    /// that report: the download itself is cut off past the same bound.
    pub async fn fetch_raw(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        if uid == 0 {
            return Err(MailError::InvalidInput);
        }
        self.select(folder, false).await?;
        let mut size = None;
        self.arm(MAX_REPLY_BYTES);
        {
            let stream = timeout(
                COMMAND_TIMEOUT,
                self.session.uid_fetch(uid.to_string(), "(UID RFC822.SIZE)"),
            )
            .await?
            .map_err(MailError::from)?;
            futures::pin_mut!(stream);
            while let Some(fetch) = timeout(COMMAND_TIMEOUT, stream.try_next())
                .await?
                .map_err(MailError::from)?
            {
                if fetch.uid == Some(uid) {
                    size = fetch.size;
                }
            }
        }
        let size = size.ok_or(MailError::NotFound)?;
        if size as usize > MAX_MESSAGE_BYTES {
            return Err(MailError::TooLarge);
        }
        let mut raw = None;
        self.arm(MAX_MESSAGE_BYTES as u64 + MAX_FETCH_OVERHEAD_BYTES);
        let downloading = async {
            let stream = timeout(
                COMMAND_TIMEOUT * 4,
                self.session.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])"),
            )
            .await?
            .map_err(MailError::from)?;
            futures::pin_mut!(stream);
            while let Some(fetch) = timeout(COMMAND_TIMEOUT * 4, stream.try_next())
                .await?
                .map_err(MailError::from)?
            {
                if fetch.uid == Some(uid) {
                    raw = fetch.body().map(<[u8]>::to_vec);
                }
            }
            Ok::<(), MailError>(())
        };
        let downloaded = timeout(FETCH_DEADLINE, downloading).await;
        // A server that sent more than it promised: too large, not a
        // protocol error. The connection is dropped through `is_poisoned`.
        if self.is_poisoned() {
            return Err(MailError::TooLarge);
        }
        downloaded??;
        self.drain_unsolicited();
        let raw = raw.ok_or(MailError::NotFound)?;
        if raw.len() > MAX_MESSAGE_BYTES {
            return Err(MailError::TooLarge);
        }
        Ok(raw)
    }

    /// Fetches and parses one message.
    pub async fn fetch_message(&mut self, folder: &str, uid: u32) -> Result<MessageBody> {
        let raw = self.fetch_raw(folder, uid).await?;
        let mut body = mime::parse_message(&raw)?;
        body.header.uid = uid;
        Ok(body)
    }

    async fn drain_stream<S, I>(stream: S) -> Result<()>
    where
        S: futures::Stream<Item = async_imap::error::Result<I>>,
    {
        futures::pin_mut!(stream);
        while timeout(COMMAND_TIMEOUT, stream.try_next())
            .await?
            .map_err(MailError::from)?
            .is_some()
        {}
        Ok(())
    }

    /// Sets or clears one flag on messages.
    pub async fn set_flag(
        &mut self,
        folder: &str,
        uids: &[u32],
        flag: FlagKind,
        on: bool,
    ) -> Result<()> {
        let set = uid_set(uids)?;
        self.select(folder, true).await?;
        self.arm(MAX_REPLY_BYTES);
        let sign = if on { '+' } else { '-' };
        let query = format!("{sign}FLAGS.SILENT ({})", flag.wire());
        let stream = timeout(COMMAND_TIMEOUT, self.session.uid_store(&set, &query))
            .await?
            .map_err(MailError::from)?;
        Self::drain_stream(stream).await?;
        self.drain_unsolicited();
        Ok(())
    }

    /// Copies messages to another folder.
    pub async fn copy_messages(&mut self, folder: &str, uids: &[u32], dest: &str) -> Result<()> {
        let set = uid_set(uids)?;
        let dest = wire_name(dest)?;
        self.select(folder, true).await?;
        self.arm(MAX_REPLY_BYTES);
        bounded(self.session.uid_copy(&set, &dest)).await?;
        self.drain_unsolicited();
        Ok(())
    }

    /// Moves messages: `UID MOVE` when offered, else copy then delete the
    /// originals by UID (which needs UIDPLUS so no other message is expunged).
    pub async fn move_messages(&mut self, folder: &str, uids: &[u32], dest: &str) -> Result<()> {
        let set = uid_set(uids)?;
        if folder == dest {
            return Err(MailError::InvalidInput);
        }
        if self.caps.move_ext {
            let dest = wire_name(dest)?;
            self.select(folder, true).await?;
            self.arm(MAX_REPLY_BYTES);
            bounded(self.session.uid_mv(&set, &dest)).await?;
            self.drain_unsolicited();
            return Ok(());
        }
        if !self.caps.uidplus {
            return Err(MailError::Unsupported);
        }
        self.copy_messages(folder, uids, dest).await?;
        self.expunge_uids(folder, uids).await
    }

    /// Permanently removes messages from a folder.
    pub async fn expunge_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        let set = uid_set(uids)?;
        self.set_flag(folder, uids, FlagKind::Deleted, true).await?;
        if self.caps.uidplus {
            self.arm(MAX_REPLY_BYTES);
            let stream = timeout(COMMAND_TIMEOUT, self.session.uid_expunge(&set))
                .await?
                .map_err(MailError::from)?;
            Self::drain_stream(stream).await?;
            self.drain_unsolicited();
        }
        Ok(())
    }

    /// `UID SEARCH` over subject, sender and text; headers of the newest hits.
    pub async fn search(&mut self, folder: &str, query: &str) -> Result<Vec<MessageHeader>> {
        let command = search_command(query)?;
        self.select(folder, false).await?;
        self.arm(MAX_SEARCH_REPLY_BYTES);
        let found = bounded(self.session.uid_search(&command)).await?;
        self.drain_unsolicited();
        let mut uids: Vec<u32> = found.into_iter().filter(|uid| *uid != 0).collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(MAX_SEARCH_HITS.min(MAX_PAGE as usize));
        let (Some(&high), Some(&low)) = (uids.first(), uids.last()) else {
            return Ok(Vec::new());
        };
        let set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let wanted: HashSet<u32> = uids.into_iter().collect();
        let mut rows = self.fetch_headers(&set, low, high).await?;
        rows.retain(|row| wanted.contains(&row.uid));
        Ok(rows)
    }

    /// Appends a message to a folder with the given flags.
    pub async fn append(&mut self, folder: &str, flags: &[&str], raw: &[u8]) -> Result<()> {
        let wire = wire_name(folder)?;
        if raw.len() > MAX_MESSAGE_BYTES {
            return Err(MailError::TooLarge);
        }
        let flags = if flags.is_empty() {
            None
        } else if flags.iter().all(|flag| {
            flag.starts_with('\\') && flag[1..].bytes().all(|b| b.is_ascii_alphabetic())
        }) {
            Some(format!("({})", flags.join(" ")))
        } else {
            return Err(MailError::InvalidInput);
        };
        self.arm(MAX_REPLY_BYTES);
        bounded(self.session.append(&wire, flags.as_deref(), None, raw)).await?;
        self.drain_unsolicited();
        Ok(())
    }

    /// Waits on IDLE in `folder` until the server reports a change, the renewal
    /// interval passes, or `stop` is notified. Returns the session for reuse.
    pub async fn watch(mut self, folder: &str, stop: &Notify) -> Result<(Self, WatchOutcome)> {
        if !self.caps.idle {
            return Err(MailError::Unsupported);
        }
        self.select(folder, false).await?;
        self.arm(MAX_REPLY_BYTES);
        let mut idle = self.session.idle();
        timeout(COMMAND_TIMEOUT, idle.init())
            .await?
            .map_err(MailError::from)?;
        let outcome = {
            let (wait, _interrupt) = idle.wait_with_timeout(IDLE_RENEW);
            tokio::select! {
                response = wait => match response.map_err(MailError::from)? {
                    async_imap::extensions::idle::IdleResponse::NewData(_) => WatchOutcome::Changed,
                    _ => WatchOutcome::Idle,
                },
                () = stop.notified() => WatchOutcome::Stopped,
            }
        };
        let session = timeout(COMMAND_TIMEOUT, idle.done())
            .await?
            .map_err(MailError::from)?;
        self.session = session;
        self.drain_unsolicited();
        Ok((self, outcome))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_sets_are_bounded_and_numeric() {
        assert_eq!(uid_set(&[3, 7]).unwrap(), "3,7");
        assert_eq!(uid_set(&[]), Err(MailError::InvalidInput));
        assert_eq!(uid_set(&[0]), Err(MailError::InvalidInput));
        assert_eq!(
            uid_set(&vec![1; MAX_UIDS_PER_COMMAND + 1]),
            Err(MailError::InvalidInput)
        );
    }

    #[test]
    fn search_quotes_and_rejects_injection() {
        assert_eq!(
            search_command("hello").unwrap(),
            "OR SUBJECT \"hello\" OR FROM \"hello\" TEXT \"hello\""
        );
        assert!(search_command("grüße")
            .unwrap()
            .starts_with("CHARSET UTF-8 "));
        assert_eq!(
            search_command("a\"b\\c").unwrap(),
            "OR SUBJECT \"a\\\"b\\\\c\" OR FROM \"a\\\"b\\\\c\" TEXT \"a\\\"b\\\\c\""
        );
        for bad in ["", "   ", "x\r\nA1 DELETE", "x\0"] {
            assert_eq!(search_command(bad), Err(MailError::InvalidInput), "{bad:?}");
        }
        assert_eq!(
            search_command(&"a".repeat(MAX_QUERY_BYTES + 1)),
            Err(MailError::InvalidInput)
        );
    }

    #[test]
    fn roles_come_from_names_when_special_use_is_missing() {
        assert_eq!(role_from_name("INBOX"), FolderRole::Inbox);
        assert_eq!(role_from_name("INBOX.Sent Items"), FolderRole::Sent);
        assert_eq!(role_from_name("Entwürfe"), FolderRole::Drafts);
        assert_eq!(role_from_name("Papierkorb"), FolderRole::Trash);
        assert_eq!(role_from_name("Projects/Alpha"), FolderRole::Other);
        // The app's other locales, with a namespace prefix or a different delimiter.
        for (name, role) in [
            ("Papelera", FolderRole::Trash),
            ("INBOX.Corbeille", FolderRole::Trash),
            ("Cestino", FolderRole::Trash),
            ("ごみ箱", FolderRole::Trash),
            ("휴지통", FolderRole::Trash),
            ("Lixeira", FolderRole::Trash),
            ("Корзина", FolderRole::Trash),
            ("Çöp Kutusu", FolderRole::Trash),
            ("Кошик", FolderRole::Trash),
            ("已删除邮件", FolderRole::Trash),
            ("垃圾桶", FolderRole::Trash),
            ("Éléments envoyés", FolderRole::Sent),
            ("Posta inviata", FolderRole::Sent),
            ("已发送", FolderRole::Sent),
            ("寄件備份", FolderRole::Sent),
            ("Borradores", FolderRole::Drafts),
            ("Rascunhos", FolderRole::Drafts),
            ("Черновики", FolderRole::Drafts),
            ("草稿", FolderRole::Drafts),
            ("Archivo", FolderRole::Archive),
            ("Архив", FolderRole::Archive),
            ("封存", FolderRole::Archive),
            ("Courrier indésirable", FolderRole::Junk),
            ("Спам", FolderRole::Junk),
            ("垃圾邮件", FolderRole::Junk),
        ] {
            assert_eq!(role_from_name(name), role, "{name}");
        }
        assert_eq!(
            role_from_attributes(&[NameAttribute::Junk]),
            Some(FolderRole::Junk)
        );
    }

    #[test]
    fn wire_names_reject_controls_and_encode_utf7() {
        assert_eq!(wire_name("Entwürfe").unwrap(), "Entw&APw-rfe");
        assert_eq!(wire_name("a\r\nb"), Err(MailError::InvalidInput));
        assert_eq!(wire_name(""), Err(MailError::InvalidInput));
    }
}
