//! Fork-owned (T-0025): the local mail cache in the meta database. Accounts
//! (no secrets), folders with UIDVALIDITY/HIGHESTMODSEQ, message headers with
//! flags and a snippet, and bodies cached on demand under a total budget.
//!
//! The store keeps plain rows; `zephium-mail` types are mapped by the caller.

use rusqlite::{params, Connection, OptionalExtension};

/// The result of a cache operation, so callers need no SQLite dependency.
pub type CacheResult<T> = rusqlite::Result<T>;

/// Accounts kept.
pub const MAX_MAIL_ACCOUNTS: usize = 16;
/// Header rows kept per folder; the lowest UIDs go first.
pub const MAX_CACHED_MESSAGES_PER_FOLDER: usize = 20_000;
/// Bytes of cached bodies in total; the least recently cached go first.
pub const MAX_CACHED_BODY_BYTES: i64 = 256 * 1024 * 1024;
/// Largest body this cache accepts for one message.
pub const MAX_CACHED_BODY_BYTES_PER_MESSAGE: usize = 4 * 1024 * 1024 + 256 * 1024;
/// Folders kept per account.
pub const MAX_CACHED_FOLDERS: usize = 512;

/// A stored account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailAccountRecord {
    /// Account id.
    pub id: String,
    /// Display name.
    pub display_name: String,
    /// Address.
    pub email: String,
    /// IMAP host.
    pub imap_host: String,
    /// IMAP port.
    pub imap_port: u16,
    /// `tls` or `starttls`.
    pub imap_security: String,
    /// SMTP host.
    pub smtp_host: String,
    /// SMTP port.
    pub smtp_port: u16,
    /// `tls` or `starttls`.
    pub smtp_security: String,
    /// Login name.
    pub username: String,
    /// `#rrggbb`.
    pub colour: String,
    /// Signature text.
    pub signature: String,
    /// Append sent mail to Sent.
    pub save_sent_copy: bool,
}

/// A stored folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailFolderRecord {
    /// Decoded path.
    pub path: String,
    /// Last segment.
    pub name: String,
    /// Hierarchy delimiter.
    pub delimiter: Option<String>,
    /// Role text (`inbox`, `sent`, ...).
    pub role: String,
    /// Unread count.
    pub unread: u32,
    /// Message count.
    pub total: u32,
    /// UIDVALIDITY.
    pub uid_validity: Option<u32>,
    /// UIDNEXT.
    pub uid_next: Option<u32>,
    /// HIGHESTMODSEQ.
    pub highest_modseq: Option<u64>,
    /// Selectable.
    pub selectable: bool,
}

/// A stored header row. Address lists and references are JSON text.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct MailMessageRecord {
    /// UID.
    pub uid: u32,
    /// Message-ID.
    pub message_id: Option<String>,
    /// Subject.
    pub subject: String,
    /// From as JSON.
    pub from_json: String,
    /// To as JSON.
    pub to_json: String,
    /// Cc as JSON.
    pub cc_json: String,
    /// Unix seconds.
    pub date: Option<i64>,
    /// Bit 0 seen, 1 flagged, 2 answered, 3 draft, 4 deleted.
    pub flags: u8,
    /// RFC822 size.
    pub size: u32,
    /// Attachment hint.
    pub has_attachments: bool,
    /// Snippet.
    pub snippet: String,
    /// In-Reply-To.
    pub in_reply_to: Option<String>,
    /// References as JSON.
    pub references_json: String,
}

/// A cached body.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct MailBodyRecord {
    /// Plain text.
    pub text: Option<String>,
    /// Raw HTML.
    pub html: Option<String>,
    /// Attachment list as JSON.
    pub attachments_json: String,
    /// A part was cut at its cap.
    pub truncated: bool,
}

/// Header rows read for one contact lookup, newest first.
pub const MAX_CONTACT_SCAN_ROWS: usize = 5_000;
/// Longest contact prefix searched, in bytes.
pub const MAX_CONTACT_PREFIX_BYTES: usize = 64;
/// Contacts returned for one lookup.
pub const MAX_CONTACT_RESULTS: u32 = 20;

/// An address seen in cached mail, for suggestions while writing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailContact {
    /// The most recent display name seen with this address.
    pub name: Option<String>,
    /// The address, as last written.
    pub email: String,
    /// Times seen in the scanned mail.
    pub count: u32,
    /// Unix seconds of the newest message it appeared in.
    pub last: i64,
}

#[derive(serde::Deserialize)]
struct StoredAddress {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: String,
}

/// The cache over one connection.
pub struct MailCache<'a> {
    conn: &'a mut Connection,
}

const MESSAGE_COLUMNS: &str = "uid, message_id, subject, from_json, to_json, cc_json, date, flags, size, has_attachments, snippet, in_reply_to, references_json";

fn message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MailMessageRecord> {
    Ok(MailMessageRecord {
        uid: row.get(0)?,
        message_id: row.get(1)?,
        subject: row.get(2)?,
        from_json: row.get(3)?,
        to_json: row.get(4)?,
        cc_json: row.get(5)?,
        date: row.get(6)?,
        flags: row.get(7)?,
        size: row.get(8)?,
        has_attachments: row.get(9)?,
        snippet: row.get(10)?,
        in_reply_to: row.get(11)?,
        references_json: row.get(12)?,
    })
}

/// The address starts with the prefix, or a word of the name does.
fn contact_matches(prefix: &str, email: &str, name: Option<&str>) -> bool {
    email.starts_with(prefix)
        || name.is_some_and(|name| {
            let name = name.to_lowercase();
            name.starts_with(prefix) || name.split_whitespace().any(|word| word.starts_with(prefix))
        })
}

impl<'a> MailCache<'a> {
    /// Wraps a meta-database connection.
    pub fn new(conn: &'a mut Connection) -> Self {
        Self { conn }
    }

    /// Accounts in the order they were added.
    pub fn accounts(&self) -> rusqlite::Result<Vec<MailAccountRecord>> {
        self.conn
            .prepare_cached(
                "SELECT id, display_name, email, imap_host, imap_port, imap_security, smtp_host,
                        smtp_port, smtp_security, username, colour, signature, save_sent_copy
                 FROM mail_accounts ORDER BY position, id",
            )?
            .query_map([], |row| {
                Ok(MailAccountRecord {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    email: row.get(2)?,
                    imap_host: row.get(3)?,
                    imap_port: row.get(4)?,
                    imap_security: row.get(5)?,
                    smtp_host: row.get(6)?,
                    smtp_port: row.get(7)?,
                    smtp_security: row.get(8)?,
                    username: row.get(9)?,
                    colour: row.get(10)?,
                    signature: row.get(11)?,
                    save_sent_copy: row.get(12)?,
                })
            })?
            .collect()
    }

    /// One account.
    pub fn account(&self, id: &str) -> rusqlite::Result<Option<MailAccountRecord>> {
        Ok(self
            .accounts()?
            .into_iter()
            .find(|account| account.id == id))
    }

    /// Adds an account or replaces the one with the same id. False when the
    /// account limit is reached for a new id.
    pub fn save_account(&mut self, account: &MailAccountRecord) -> rusqlite::Result<bool> {
        let tx = self.conn.transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT position FROM mail_accounts WHERE id = ?1",
                [&account.id],
                |row| row.get(0),
            )
            .optional()?;
        let position = match existing {
            Some(position) => position,
            None => {
                let count: i64 =
                    tx.query_row("SELECT count(*) FROM mail_accounts", [], |r| r.get(0))?;
                if count >= MAX_MAIL_ACCOUNTS as i64 {
                    return Ok(false);
                }
                tx.query_row(
                    "SELECT coalesce(max(position) + 1, 0) FROM mail_accounts",
                    [],
                    |r| r.get(0),
                )?
            }
        };
        tx.execute(
            "INSERT INTO mail_accounts(id, position, display_name, email, imap_host, imap_port,
                 imap_security, smtp_host, smtp_port, smtp_security, username, colour, signature, save_sent_copy)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id) DO UPDATE SET
                 display_name = excluded.display_name, email = excluded.email,
                 imap_host = excluded.imap_host, imap_port = excluded.imap_port,
                 imap_security = excluded.imap_security, smtp_host = excluded.smtp_host,
                 smtp_port = excluded.smtp_port, smtp_security = excluded.smtp_security,
                 username = excluded.username, colour = excluded.colour,
                 signature = excluded.signature, save_sent_copy = excluded.save_sent_copy",
            params![
                account.id,
                position,
                account.display_name,
                account.email,
                account.imap_host,
                account.imap_port,
                account.imap_security,
                account.smtp_host,
                account.smtp_port,
                account.smtp_security,
                account.username,
                account.colour,
                account.signature,
                account.save_sent_copy,
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Removes an account and everything cached for it.
    pub fn remove_account(&mut self, id: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM mail_messages WHERE account_id = ?1", [id])?;
        tx.execute("DELETE FROM mail_folders WHERE account_id = ?1", [id])?;
        let removed = tx.execute("DELETE FROM mail_accounts WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(removed > 0)
    }

    /// Folders of an account.
    pub fn folders(&self, account_id: &str) -> rusqlite::Result<Vec<MailFolderRecord>> {
        self.conn
            .prepare_cached(
                "SELECT path, name, delimiter, role, unread, total, uid_validity, uid_next,
                        highest_modseq, selectable
                 FROM mail_folders WHERE account_id = ?1 ORDER BY path",
            )?
            .query_map([account_id], |row| {
                Ok(MailFolderRecord {
                    path: row.get(0)?,
                    name: row.get(1)?,
                    delimiter: row.get(2)?,
                    role: row.get(3)?,
                    unread: row.get(4)?,
                    total: row.get(5)?,
                    uid_validity: row.get(6)?,
                    uid_next: row.get(7)?,
                    highest_modseq: row
                        .get::<_, Option<i64>>(8)?
                        .map(|v| u64::try_from(v).unwrap_or(0)),
                    selectable: row.get(9)?,
                })
            })?
            .collect()
    }

    /// Replaces the folder list. A folder whose UIDVALIDITY changed loses its
    /// cached messages (their UIDs no longer mean anything); folders the
    /// server no longer lists are dropped with their messages.
    pub fn replace_folders(
        &mut self,
        account_id: &str,
        folders: &[MailFolderRecord],
    ) -> rusqlite::Result<()> {
        if folders.len() > MAX_CACHED_FOLDERS {
            return Err(rusqlite::Error::InvalidParameterCount(
                folders.len(),
                MAX_CACHED_FOLDERS,
            ));
        }
        let tx = self.conn.transaction()?;
        let known: std::collections::HashMap<String, Option<u32>> = tx
            .prepare("SELECT path, uid_validity FROM mail_folders WHERE account_id = ?1")?
            .query_map([account_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for folder in folders {
            if let Some(Some(old)) = known.get(&folder.path) {
                if folder.uid_validity.is_some_and(|new| new != *old) {
                    tx.execute(
                        "DELETE FROM mail_messages WHERE account_id = ?1 AND folder = ?2",
                        params![account_id, folder.path],
                    )?;
                }
            }
            tx.execute(
                "INSERT INTO mail_folders(account_id, path, name, delimiter, role, unread, total,
                     uid_validity, uid_next, highest_modseq, selectable)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(account_id, path) DO UPDATE SET
                     name = excluded.name, delimiter = excluded.delimiter, role = excluded.role,
                     unread = excluded.unread, total = excluded.total,
                     uid_validity = excluded.uid_validity, uid_next = excluded.uid_next,
                     highest_modseq = excluded.highest_modseq, selectable = excluded.selectable",
                params![
                    account_id,
                    folder.path,
                    folder.name,
                    folder.delimiter,
                    folder.role,
                    folder.unread,
                    folder.total,
                    folder.uid_validity,
                    folder.uid_next,
                    folder
                        .highest_modseq
                        .map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
                    folder.selectable,
                ],
            )?;
        }
        let keep: std::collections::HashSet<&str> =
            folders.iter().map(|f| f.path.as_str()).collect();
        for path in known.keys().filter(|path| !keep.contains(path.as_str())) {
            tx.execute(
                "DELETE FROM mail_messages WHERE account_id = ?1 AND folder = ?2",
                params![account_id, path],
            )?;
            tx.execute(
                "DELETE FROM mail_folders WHERE account_id = ?1 AND path = ?2",
                params![account_id, path],
            )?;
        }
        tx.commit()
    }

    /// Stores a page of headers read at `uid_validity`. Bodies already cached
    /// for a UID survive a header refresh.
    pub fn store_headers(
        &mut self,
        account_id: &str,
        folder: &str,
        uid_validity: Option<u32>,
        rows: &[MailMessageRecord],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        if let Some(new) = uid_validity {
            let old: Option<Option<u32>> = tx
                .query_row(
                    "SELECT uid_validity FROM mail_folders WHERE account_id = ?1 AND path = ?2",
                    params![account_id, folder],
                    |row| row.get(0),
                )
                .optional()?;
            if matches!(old, Some(Some(old)) if old != new) {
                tx.execute(
                    "DELETE FROM mail_messages WHERE account_id = ?1 AND folder = ?2",
                    params![account_id, folder],
                )?;
            }
            tx.execute(
                "UPDATE mail_folders SET uid_validity = ?3 WHERE account_id = ?1 AND path = ?2",
                params![account_id, folder, new],
            )?;
        }
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO mail_messages(account_id, folder, uid, message_id, subject, from_json,
                     to_json, cc_json, date, flags, size, has_attachments, snippet, in_reply_to, references_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(account_id, folder, uid) DO UPDATE SET
                     message_id = excluded.message_id, subject = excluded.subject,
                     from_json = excluded.from_json, to_json = excluded.to_json,
                     cc_json = excluded.cc_json, date = excluded.date, flags = excluded.flags,
                     size = excluded.size, has_attachments = excluded.has_attachments,
                     snippet = excluded.snippet, in_reply_to = excluded.in_reply_to,
                     references_json = excluded.references_json",
            )?;
            for row in rows {
                insert.execute(params![
                    account_id,
                    folder,
                    row.uid,
                    row.message_id,
                    row.subject,
                    row.from_json,
                    row.to_json,
                    row.cc_json,
                    row.date,
                    row.flags,
                    row.size,
                    row.has_attachments,
                    row.snippet,
                    row.in_reply_to,
                    row.references_json,
                ])?;
            }
        }
        tx.execute(
            "DELETE FROM mail_messages WHERE account_id = ?1 AND folder = ?2 AND uid IN (
                 SELECT uid FROM mail_messages WHERE account_id = ?1 AND folder = ?2
                 ORDER BY uid DESC LIMIT -1 OFFSET ?3)",
            params![account_id, folder, MAX_CACHED_MESSAGES_PER_FOLDER as i64],
        )?;
        tx.commit()
    }

    /// Cached headers, newest UID first; `before_uid` is exclusive.
    pub fn cached_headers(
        &self,
        account_id: &str,
        folder: &str,
        before_uid: Option<u32>,
        limit: u32,
    ) -> rusqlite::Result<Vec<MailMessageRecord>> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM mail_messages
             WHERE account_id = ?1 AND folder = ?2 AND uid < ?3
             ORDER BY uid DESC LIMIT ?4"
        );
        self.conn
            .prepare_cached(&sql)?
            .query_map(
                params![
                    account_id,
                    folder,
                    before_uid.map_or(i64::MAX, i64::from),
                    i64::from(limit.min(200))
                ],
                message_row,
            )?
            .collect()
    }

    /// Addresses from cached mail that start with `prefix` (on the address or
    /// on a word of the name), best first: recipients of sent mail and senders
    /// of received mail, weighted by how often and how recently they appear.
    /// Reads only the cache (at most [`MAX_CONTACT_SCAN_ROWS`] header rows) and
    /// returns at most `limit.min(MAX_CONTACT_RESULTS)` entries. Junk, trash
    /// and the accounts' own addresses are left out.
    pub fn contacts(&self, prefix: &str, limit: u32) -> rusqlite::Result<Vec<MailContact>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            });
        self.contacts_at(prefix, limit, now)
    }

    /// [`Self::contacts`] with the clock given, for tests.
    pub fn contacts_at(
        &self,
        prefix: &str,
        limit: u32,
        now: i64,
    ) -> rusqlite::Result<Vec<MailContact>> {
        let wanted = prefix.trim().to_lowercase();
        if wanted.is_empty()
            || wanted.len() > MAX_CONTACT_PREFIX_BYTES
            || wanted.chars().any(char::is_control)
        {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, MAX_CONTACT_RESULTS) as usize;
        let own: Vec<String> = self
            .conn
            .prepare_cached("SELECT lower(email) FROM mail_accounts")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        // A pre-filter only: the exact test below decides.
        let like = format!(
            "%{}%",
            wanted
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut statement = self.conn.prepare_cached(
            "SELECT m.from_json, m.to_json, m.cc_json, m.date, f.role
             FROM mail_messages m
             JOIN mail_folders f ON f.account_id = m.account_id AND f.path = m.folder
             WHERE f.role NOT IN ('junk', 'trash')
               AND (m.from_json LIKE ?1 ESCAPE '\\' OR m.to_json LIKE ?1 ESCAPE '\\'
                    OR m.cc_json LIKE ?1 ESCAPE '\\')
             ORDER BY m.date DESC LIMIT ?2",
        )?;
        struct Seen {
            contact: MailContact,
            score: f64,
        }
        let mut seen: Vec<Seen> = Vec::new();
        let mut rows = statement.query(params![like, MAX_CONTACT_SCAN_ROWS as i64])?;
        while let Some(row) = rows.next()? {
            let (from, to, cc): (String, String, String) = (row.get(0)?, row.get(1)?, row.get(2)?);
            let date: Option<i64> = row.get(3)?;
            let role: String = row.get(4)?;
            let sent = role == "sent";
            let lists: &[&String] = if sent { &[&to, &cc] } else { &[&from] };
            let when = date.unwrap_or(0);
            let age_days = (now.saturating_sub(when).max(0) as f64) / 86_400.0;
            // Each sighting counts one, fading with age; mail I wrote counts double.
            let weight = (if sent { 2.0 } else { 1.0 }) / (1.0 + age_days / 90.0);
            for list in lists {
                let Ok(addresses) = serde_json::from_str::<Vec<StoredAddress>>(list) else {
                    continue;
                };
                for address in addresses {
                    let email = address.email.trim();
                    let lower = email.to_lowercase();
                    if email.is_empty()
                        || email.len() > 254
                        || email.chars().any(|c| c.is_control() || c.is_whitespace())
                        || own.contains(&lower)
                        || !contact_matches(&wanted, &lower, address.name.as_deref())
                    {
                        continue;
                    }
                    let name = address
                        .name
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty() && name.len() <= 256)
                        .map(str::to_owned);
                    match seen
                        .iter_mut()
                        .find(|entry| entry.contact.email.to_lowercase() == lower)
                    {
                        Some(entry) => {
                            entry.contact.count = entry.contact.count.saturating_add(1);
                            entry.contact.last = entry.contact.last.max(when);
                            entry.score += weight;
                            // Rows arrive newest first: keep the newest name.
                            if entry.contact.name.is_none() {
                                entry.contact.name = name;
                            }
                        }
                        None => seen.push(Seen {
                            contact: MailContact {
                                name,
                                email: email.to_owned(),
                                count: 1,
                                last: when,
                            },
                            score: weight,
                        }),
                    }
                }
            }
        }
        seen.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(b.contact.last.cmp(&a.contact.last))
                .then_with(|| a.contact.email.cmp(&b.contact.email))
        });
        Ok(seen
            .into_iter()
            .take(limit)
            .map(|entry| entry.contact)
            .collect())
    }

    /// Updates the cached flags of one message.
    pub fn set_flags(
        &mut self,
        account_id: &str,
        folder: &str,
        uid: u32,
        flags: u8,
    ) -> rusqlite::Result<bool> {
        Ok(self.conn.execute(
            "UPDATE mail_messages SET flags = ?4 WHERE account_id = ?1 AND folder = ?2 AND uid = ?3",
            params![account_id, folder, uid, flags],
        )? > 0)
    }

    /// Sets or clears one flag bit on cached messages, leaving the others.
    pub fn update_flag_bit(
        &mut self,
        account_id: &str,
        folder: &str,
        uids: &[u32],
        bit: u8,
        on: bool,
    ) -> rusqlite::Result<usize> {
        let sql = if on {
            "UPDATE mail_messages SET flags = flags | ?4 WHERE account_id = ?1 AND folder = ?2 AND uid = ?3"
        } else {
            "UPDATE mail_messages SET flags = flags & ~?4 WHERE account_id = ?1 AND folder = ?2 AND uid = ?3"
        };
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        for uid in uids {
            changed += tx.execute(sql, params![account_id, folder, uid, bit])?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Drops messages from the cache (after a move, delete or expunge).
    pub fn remove_messages(
        &mut self,
        account_id: &str,
        folder: &str,
        uids: &[u32],
    ) -> rusqlite::Result<usize> {
        let tx = self.conn.transaction()?;
        let mut removed = 0;
        for uid in uids {
            removed += tx.execute(
                "DELETE FROM mail_messages WHERE account_id = ?1 AND folder = ?2 AND uid = ?3",
                params![account_id, folder, uid],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Caches a body, then evicts the least recently cached bodies until the
    /// total fits [`MAX_CACHED_BODY_BYTES`]. False when it is too large or the
    /// header row is not cached.
    pub fn store_body(
        &mut self,
        account_id: &str,
        folder: &str,
        uid: u32,
        body: &MailBodyRecord,
        now: i64,
    ) -> rusqlite::Result<bool> {
        let bytes = body.text.as_ref().map_or(0, String::len)
            + body.html.as_ref().map_or(0, String::len)
            + body.attachments_json.len();
        if bytes == 0 || bytes > MAX_CACHED_BODY_BYTES_PER_MESSAGE {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE mail_messages SET body_text = ?4, body_html = ?5, body_attachments_json = ?6,
                 body_truncated = ?7, body_bytes = ?8, body_cached_at = ?9
             WHERE account_id = ?1 AND folder = ?2 AND uid = ?3",
            params![
                account_id,
                folder,
                uid,
                body.text,
                body.html,
                body.attachments_json,
                body.truncated,
                bytes as i64,
                now,
            ],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        loop {
            let total: i64 = tx.query_row(
                "SELECT coalesce(sum(body_bytes), 0) FROM mail_messages",
                [],
                |r| r.get(0),
            )?;
            if total <= MAX_CACHED_BODY_BYTES {
                break;
            }
            let evicted = tx.execute(
                "UPDATE mail_messages SET body_text = NULL, body_html = NULL, body_attachments_json = NULL,
                     body_truncated = 0, body_bytes = 0, body_cached_at = NULL
                 WHERE (account_id, folder, uid) = (
                     SELECT account_id, folder, uid FROM mail_messages WHERE body_bytes > 0
                     ORDER BY body_cached_at, account_id, folder, uid LIMIT 1)",
                [],
            )?;
            if evicted == 0 {
                break;
            }
        }
        tx.commit()?;
        Ok(true)
    }

    /// The cached body of a message, if any.
    pub fn cached_body(
        &self,
        account_id: &str,
        folder: &str,
        uid: u32,
    ) -> rusqlite::Result<Option<MailBodyRecord>> {
        let row: Option<(MailBodyRecord, i64)> = self
            .conn
            .query_row(
                "SELECT body_text, body_html, body_attachments_json, body_truncated, body_bytes
                 FROM mail_messages WHERE account_id = ?1 AND folder = ?2 AND uid = ?3",
                params![account_id, folder, uid],
                |row| {
                    Ok((
                        MailBodyRecord {
                            text: row.get(0)?,
                            html: row.get(1)?,
                            attachments_json: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                            truncated: row.get(3)?,
                        },
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.and_then(|(record, bytes)| (bytes > 0).then_some(record)))
    }

    /// Total bytes of cached bodies.
    pub fn cached_body_bytes(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT coalesce(sum(body_bytes), 0) FROM mail_messages",
            [],
            |r| r.get(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations;

    fn meta() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::apply(&mut conn, migrations::META).unwrap();
        conn
    }

    fn account(id: &str) -> MailAccountRecord {
        MailAccountRecord {
            id: id.into(),
            display_name: "Ann".into(),
            email: "ann@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: "tls".into(),
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_security: "starttls".into(),
            username: "ann".into(),
            colour: "#336699".into(),
            signature: "-- Ann".into(),
            save_sent_copy: true,
        }
    }

    fn folder(path: &str, validity: u32) -> MailFolderRecord {
        MailFolderRecord {
            path: path.into(),
            name: path.into(),
            delimiter: Some("/".into()),
            role: if path == "INBOX" { "inbox" } else { "other" }.into(),
            unread: 2,
            total: 9,
            uid_validity: Some(validity),
            uid_next: Some(10),
            highest_modseq: Some(77),
            selectable: true,
        }
    }

    fn message(uid: u32) -> MailMessageRecord {
        MailMessageRecord {
            uid,
            message_id: Some(format!("m{uid}@x")),
            subject: format!("Subject {uid}"),
            from_json: r#"[{"name":"Ann","email":"ann@example.com"}]"#.into(),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            date: Some(1_731_499_200),
            flags: 1,
            size: 1234,
            has_attachments: false,
            snippet: "hello".into(),
            in_reply_to: None,
            references_json: "[]".into(),
        }
    }

    #[test]
    fn the_migration_creates_the_mail_tables_and_roundtrips_accounts() {
        let mut conn = meta();
        let mut cache = MailCache::new(&mut conn);
        assert!(cache.accounts().unwrap().is_empty());
        assert!(cache.save_account(&account("a1")).unwrap());
        assert!(cache.save_account(&account("a2")).unwrap());
        let mut changed = account("a1");
        changed.signature = "new".into();
        changed.save_sent_copy = false;
        assert!(cache.save_account(&changed).unwrap());
        let accounts = cache.accounts().unwrap();
        assert_eq!(
            accounts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        assert_eq!(accounts[0], changed);
        assert_eq!(
            cache.account("a2").unwrap().unwrap().smtp_security,
            "starttls"
        );
        assert!(cache.account("zz").unwrap().is_none());
    }

    fn sent_message(uid: u32, to: &str, date: i64) -> MailMessageRecord {
        MailMessageRecord {
            to_json: to.into(),
            from_json: r#"[{"name":"Ann","email":"ann@example.com"}]"#.into(),
            date: Some(date),
            ..message(uid)
        }
    }

    fn received(uid: u32, from: &str, date: i64) -> MailMessageRecord {
        MailMessageRecord {
            from_json: from.into(),
            date: Some(date),
            ..message(uid)
        }
    }

    fn contact_cache(conn: &mut Connection) {
        let mut cache = MailCache::new(conn);
        cache.save_account(&account("a1")).unwrap();
        let mut sent = folder("Sent", 1);
        sent.role = "sent".into();
        let mut junk = folder("Junk", 1);
        junk.role = "junk".into();
        cache
            .replace_folders("a1", &[folder("INBOX", 1), sent, junk])
            .unwrap();
    }

    const NOW: i64 = 1_731_499_200;
    const DAY: i64 = 86_400;

    #[test]
    fn contacts_rank_by_frequency_and_recency_and_match_names_and_addresses() {
        let mut conn = meta();
        contact_cache(&mut conn);
        let mut cache = MailCache::new(&mut conn);
        cache
            .store_headers(
                "a1",
                "Sent",
                Some(1),
                &[
                    sent_message(
                        1,
                        r#"[{"name":"Bob Brown","email":"bob@example.com"}]"#,
                        NOW - DAY,
                    ),
                    sent_message(
                        2,
                        r#"[{"name":"Bob Brown","email":"bob@example.com"}]"#,
                        NOW - 2 * DAY,
                    ),
                    sent_message(
                        3,
                        r#"[{"name":null,"email":"bea@example.com"}]"#,
                        NOW - 3 * DAY,
                    ),
                ],
            )
            .unwrap();
        cache
            .store_headers(
                "a1",
                "INBOX",
                Some(1),
                &[
                    received(
                        1,
                        r#"[{"name":"Bea Old","email":"bea@example.com"}]"#,
                        NOW - 400 * DAY,
                    ),
                    received(
                        2,
                        r#"[{"name":"Bernd","email":"bernd@example.com"}]"#,
                        NOW - 10 * DAY,
                    ),
                    received(
                        3,
                        r#"[{"name":"Carl","email":"carl@example.com"}]"#,
                        NOW - DAY,
                    ),
                    received(
                        4,
                        r#"[{"name":"Bob Brown","email":"BOB@example.com"}]"#,
                        NOW - 5 * DAY,
                    ),
                ],
            )
            .unwrap();
        let found = cache.contacts_at("b", 10, NOW).unwrap();
        let emails: Vec<&str> = found.iter().map(|c| c.email.as_str()).collect();
        // Bob: two sent + one received (case-folded together); Bea: sent + old; Bernd: one received.
        assert_eq!(
            emails,
            ["bob@example.com", "bea@example.com", "bernd@example.com"]
        );
        assert_eq!(found[0].count, 3);
        assert_eq!(found[0].name.as_deref(), Some("Bob Brown"));
        assert_eq!(found[0].last, NOW - DAY);
        // A word of the name matches too, and Bea keeps the name from the newest row that had one.
        let found = cache.contacts_at("old", 10, NOW).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name.as_deref(), Some("Bea Old"));
        // Prefix on the address.
        assert_eq!(
            cache.contacts_at("CARL@", 10, NOW).unwrap()[0].email,
            "carl@example.com"
        );
        // Substrings inside a word do not match.
        assert!(cache.contacts_at("rown", 10, NOW).unwrap().is_empty());
    }

    #[test]
    fn contacts_are_bounded_and_skip_junk_own_addresses_and_bad_input() {
        let mut conn = meta();
        contact_cache(&mut conn);
        let mut cache = MailCache::new(&mut conn);
        let many: Vec<MailMessageRecord> = (1..=30)
            .map(|uid| {
                received(
                    uid,
                    &format!(r#"[{{"name":"P {uid}","email":"p{uid}@example.com"}}]"#),
                    NOW - i64::from(uid) * DAY,
                )
            })
            .collect();
        cache.store_headers("a1", "INBOX", Some(1), &many).unwrap();
        cache
            .store_headers(
                "a1",
                "Junk",
                Some(1),
                &[received(
                    1,
                    r#"[{"name":"P Spam","email":"pspam@example.com"}]"#,
                    NOW,
                )],
            )
            .unwrap();
        cache
            .store_headers(
                "a1",
                "Sent",
                Some(1),
                &[sent_message(
                    1,
                    r#"[{"name":"Me","email":"ann@example.com"}]"#,
                    NOW,
                )],
            )
            .unwrap();
        let found = cache.contacts_at("p", 1_000, NOW).unwrap();
        assert_eq!(found.len(), MAX_CONTACT_RESULTS as usize);
        assert_eq!(found[0].email, "p1@example.com");
        assert!(found.iter().all(|c| c.email != "pspam@example.com"));
        assert_eq!(cache.contacts_at("p", 3, NOW).unwrap().len(), 3);
        assert_eq!(cache.contacts_at("p", 0, NOW).unwrap().len(), 1);
        // Own address, empty, control and oversized prefixes, SQL wildcards.
        assert!(cache.contacts_at("ann@", 10, NOW).unwrap().is_empty());
        assert!(cache.contacts_at("  ", 10, NOW).unwrap().is_empty());
        assert!(cache.contacts_at("p\n", 10, NOW).unwrap().len() <= 20);
        assert!(cache.contacts_at("p\u{0}x", 10, NOW).unwrap().is_empty());
        assert!(cache
            .contacts_at(&"p".repeat(MAX_CONTACT_PREFIX_BYTES + 1), 10, NOW)
            .unwrap()
            .is_empty());
        assert!(cache.contacts_at("%", 10, NOW).unwrap().is_empty());
        assert!(cache.contacts_at("_", 10, NOW).unwrap().is_empty());
    }

    #[test]
    fn contacts_survive_malformed_stored_json_and_hostile_addresses() {
        let mut conn = meta();
        contact_cache(&mut conn);
        let mut cache = MailCache::new(&mut conn);
        cache
            .store_headers(
                "a1",
                "INBOX",
                Some(1),
                &[
                    received(1, r#"{"not":"a list"}"#, NOW),
                    received(2, r#"[{"name":"X","email":"x y@example.com"}]"#, NOW),
                    received(3, r#"[{"name":"X","email":"x\u0007@example.com"}]"#, NOW),
                    received(
                        4,
                        r#"[{"name":"Xavier","email":"xavier@example.com"}]"#,
                        NOW,
                    ),
                ],
            )
            .unwrap();
        let found = cache.contacts_at("x", 10, NOW).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].email, "xavier@example.com");
    }

    #[test]
    fn the_schema_holds_no_password_column() {
        let conn = meta();
        for table in ["mail_accounts", "mail_folders", "mail_messages"] {
            let columns: Vec<String> = conn
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(
                !columns
                    .iter()
                    .any(|c| c.contains("pass") || c.contains("secret") || c.contains("token")),
                "{table}: {columns:?}"
            );
        }
    }

    #[test]
    fn the_account_limit_and_check_constraints_refuse_bad_rows() {
        let mut conn = meta();
        let mut cache = MailCache::new(&mut conn);
        for n in 0..MAX_MAIL_ACCOUNTS {
            assert!(cache.save_account(&account(&format!("a{n}"))).unwrap());
        }
        assert!(!cache.save_account(&account("one-too-many")).unwrap());
        assert!(
            cache.save_account(&account("a0")).unwrap(),
            "replacing is not adding"
        );
        assert!(cache.remove_account("a1").unwrap());
        assert!(cache.save_account(&account("../evil")).is_err());
        let mut bad = account("ok");
        bad.imap_security = "plain".into();
        assert!(
            cache.save_account(&bad).is_err(),
            "plain text is not storable"
        );
    }

    #[test]
    fn folders_replace_in_place_and_a_validity_change_drops_cached_messages() {
        let mut conn = meta();
        let mut cache = MailCache::new(&mut conn);
        cache.save_account(&account("a1")).unwrap();
        cache
            .replace_folders("a1", &[folder("INBOX", 7), folder("Old", 1)])
            .unwrap();
        cache
            .store_headers("a1", "INBOX", Some(7), &[message(1), message(2)])
            .unwrap();
        cache
            .store_headers("a1", "Old", Some(1), &[message(1)])
            .unwrap();
        assert_eq!(
            cache.cached_headers("a1", "INBOX", None, 10).unwrap().len(),
            2
        );
        // Same validity keeps messages; "Old" disappears with its messages.
        cache.replace_folders("a1", &[folder("INBOX", 7)]).unwrap();
        assert_eq!(cache.folders("a1").unwrap().len(), 1);
        assert_eq!(
            cache.cached_headers("a1", "INBOX", None, 10).unwrap().len(),
            2
        );
        assert!(cache
            .cached_headers("a1", "Old", None, 10)
            .unwrap()
            .is_empty());
        // A new UIDVALIDITY invalidates every cached UID.
        cache.replace_folders("a1", &[folder("INBOX", 8)]).unwrap();
        assert!(cache
            .cached_headers("a1", "INBOX", None, 10)
            .unwrap()
            .is_empty());
        let stored = &cache.folders("a1").unwrap()[0];
        assert_eq!(
            (stored.uid_validity, stored.highest_modseq, stored.unread),
            (Some(8), Some(77), 2)
        );
    }

    #[test]
    fn headers_page_flag_and_remove() {
        let mut conn = meta();
        let mut cache = MailCache::new(&mut conn);
        cache.save_account(&account("a1")).unwrap();
        cache.replace_folders("a1", &[folder("INBOX", 7)]).unwrap();
        let rows: Vec<_> = (1..=5).map(message).collect();
        cache.store_headers("a1", "INBOX", Some(7), &rows).unwrap();
        let page = cache.cached_headers("a1", "INBOX", Some(4), 2).unwrap();
        assert_eq!(page.iter().map(|m| m.uid).collect::<Vec<_>>(), [3, 2]);
        assert_eq!(page[0], rows[2]);
        assert!(cache.set_flags("a1", "INBOX", 3, 3).unwrap());
        assert_eq!(
            cache.cached_headers("a1", "INBOX", Some(4), 1).unwrap()[0].flags,
            3
        );
        assert!(!cache.set_flags("a1", "INBOX", 99, 1).unwrap());
        assert_eq!(
            cache
                .update_flag_bit("a1", "INBOX", &[3, 4, 99], 2, true)
                .unwrap(),
            2
        );
        assert_eq!(
            cache.cached_headers("a1", "INBOX", Some(4), 1).unwrap()[0].flags,
            3
        );
        assert_eq!(
            cache
                .update_flag_bit("a1", "INBOX", &[3], 1, false)
                .unwrap(),
            1
        );
        assert_eq!(
            cache.cached_headers("a1", "INBOX", Some(4), 1).unwrap()[0].flags,
            2
        );
        assert_eq!(
            cache.remove_messages("a1", "INBOX", &[1, 2, 77]).unwrap(),
            2
        );
        assert_eq!(
            cache.cached_headers("a1", "INBOX", None, 10).unwrap().len(),
            3
        );
        // A hostile header cannot exceed its column bound.
        let mut huge = message(9);
        huge.subject = "x".repeat(2000);
        assert!(cache
            .store_headers("a1", "INBOX", Some(7), &[huge])
            .is_err());
        let mut bad_json = message(10);
        bad_json.from_json = "not json".into();
        assert!(cache
            .store_headers("a1", "INBOX", Some(7), &[bad_json])
            .is_err());
    }

    #[test]
    fn bodies_cache_on_demand_survive_header_refresh_and_evict_oldest_first() {
        let mut conn = meta();
        let mut cache = MailCache::new(&mut conn);
        cache.save_account(&account("a1")).unwrap();
        cache.replace_folders("a1", &[folder("INBOX", 7)]).unwrap();
        cache
            .store_headers("a1", "INBOX", Some(7), &[message(1), message(2)])
            .unwrap();
        let body = MailBodyRecord {
            text: Some("hello".into()),
            html: Some("<p>hello</p>".into()),
            attachments_json: "[]".into(),
            truncated: false,
        };
        assert!(cache.store_body("a1", "INBOX", 1, &body, 100).unwrap());
        assert_eq!(
            cache.cached_body("a1", "INBOX", 1).unwrap(),
            Some(body.clone())
        );
        assert_eq!(cache.cached_body("a1", "INBOX", 2).unwrap(), None);
        cache
            .store_headers("a1", "INBOX", Some(7), &[message(1)])
            .unwrap();
        assert_eq!(
            cache.cached_body("a1", "INBOX", 1).unwrap(),
            Some(body.clone()),
            "refresh keeps the body"
        );
        assert!(
            !cache.store_body("a1", "INBOX", 99, &body, 100).unwrap(),
            "no header row, no body"
        );
        let empty = MailBodyRecord::default();
        assert!(!cache.store_body("a1", "INBOX", 1, &empty, 100).unwrap());
        let big = MailBodyRecord {
            text: Some("x".repeat(MAX_CACHED_BODY_BYTES_PER_MESSAGE + 1)),
            ..MailBodyRecord::default()
        };
        assert!(!cache.store_body("a1", "INBOX", 1, &big, 100).unwrap());
        assert_eq!(cache.cached_body_bytes().unwrap(), 5 + 12 + 2);
    }

    #[test]
    fn the_body_budget_evicts_the_least_recently_cached() {
        let mut conn = meta();
        let mut cache = MailCache::new(&mut conn);
        cache.save_account(&account("a1")).unwrap();
        cache.replace_folders("a1", &[folder("INBOX", 7)]).unwrap();
        let rows: Vec<_> = (1..=70).map(message).collect();
        cache.store_headers("a1", "INBOX", Some(7), &rows).unwrap();
        let chunk = MailBodyRecord {
            text: Some("y".repeat(2 * 1024 * 1024)),
            html: Some("z".repeat(2 * 1024 * 1024)),
            attachments_json: "[]".into(),
            truncated: false,
        };
        for uid in 1..=70u32 {
            assert!(cache
                .store_body("a1", "INBOX", uid, &chunk, i64::from(uid))
                .unwrap());
        }
        assert!(cache.cached_body_bytes().unwrap() <= MAX_CACHED_BODY_BYTES);
        assert_eq!(
            cache.cached_body("a1", "INBOX", 1).unwrap(),
            None,
            "oldest evicted"
        );
        assert!(
            cache.cached_body("a1", "INBOX", 70).unwrap().is_some(),
            "newest kept"
        );
        assert_eq!(
            cache
                .cached_headers("a1", "INBOX", None, 200)
                .unwrap()
                .len(),
            70,
            "headers stay"
        );
    }

    #[test]
    fn removing_an_account_removes_everything_cached_for_it() {
        let mut conn = meta();
        let mut cache = MailCache::new(&mut conn);
        cache.save_account(&account("a1")).unwrap();
        cache.save_account(&account("a2")).unwrap();
        for id in ["a1", "a2"] {
            cache.replace_folders(id, &[folder("INBOX", 7)]).unwrap();
            cache
                .store_headers(id, "INBOX", Some(7), &[message(1)])
                .unwrap();
        }
        assert!(cache.remove_account("a1").unwrap());
        assert!(!cache.remove_account("a1").unwrap());
        assert!(cache.folders("a1").unwrap().is_empty());
        assert!(cache
            .cached_headers("a1", "INBOX", None, 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            cache.cached_headers("a2", "INBOX", None, 10).unwrap().len(),
            1
        );
    }

    #[test]
    fn the_new_migration_is_forward_only_and_validates_its_manifest() {
        let mut conn = meta();
        let latest = migrations::META.last().unwrap().version;
        assert!(latest >= 29);
        // Re-applying is a no-op and the schema still matches its version.
        migrations::apply(&mut conn, migrations::META).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, latest);
        migrations::validate_current(&conn, migrations::META).unwrap();
    }
}
