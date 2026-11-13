-- Fork (T-0025): local mail cache. Application-wide, in the meta database.
-- No secret lives here: passwords are only in the OS vault. Bounds mirror
-- zephium-mail's limits; row counts and the body budget are held by the writes.

CREATE TABLE mail_accounts (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(CAST(id AS BLOB)) BETWEEN 1 AND 40 AND id NOT GLOB '*[^A-Za-z0-9-]*'
    ),
    position INTEGER NOT NULL CHECK (position >= 0),
    display_name TEXT NOT NULL CHECK (length(CAST(display_name AS BLOB)) <= 8192),
    email TEXT NOT NULL CHECK (length(CAST(email AS BLOB)) BETWEEN 3 AND 254),
    imap_host TEXT NOT NULL CHECK (length(CAST(imap_host AS BLOB)) BETWEEN 1 AND 253),
    imap_port INTEGER NOT NULL CHECK (imap_port BETWEEN 1 AND 65535),
    imap_security TEXT NOT NULL CHECK (imap_security IN ('tls', 'starttls')),
    smtp_host TEXT NOT NULL CHECK (length(CAST(smtp_host AS BLOB)) BETWEEN 1 AND 253),
    smtp_port INTEGER NOT NULL CHECK (smtp_port BETWEEN 1 AND 65535),
    smtp_security TEXT NOT NULL CHECK (smtp_security IN ('tls', 'starttls')),
    username TEXT NOT NULL CHECK (length(CAST(username AS BLOB)) BETWEEN 1 AND 320),
    colour TEXT NOT NULL CHECK (length(colour) = 7),
    signature TEXT NOT NULL CHECK (length(CAST(signature AS BLOB)) <= 8192),
    save_sent_copy INTEGER NOT NULL CHECK (save_sent_copy IN (0, 1))
) STRICT;

CREATE TABLE mail_folders (
    account_id TEXT NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    path TEXT NOT NULL CHECK (length(CAST(path AS BLOB)) BETWEEN 1 AND 2048),
    name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) <= 2048),
    delimiter TEXT CHECK (delimiter IS NULL OR length(CAST(delimiter AS BLOB)) BETWEEN 1 AND 4),
    role TEXT NOT NULL CHECK (role IN ('inbox', 'sent', 'drafts', 'trash', 'archive', 'junk', 'other')),
    unread INTEGER NOT NULL CHECK (unread BETWEEN 0 AND 4294967295),
    total INTEGER NOT NULL CHECK (total BETWEEN 0 AND 4294967295),
    uid_validity INTEGER CHECK (uid_validity IS NULL OR uid_validity BETWEEN 0 AND 4294967295),
    uid_next INTEGER CHECK (uid_next IS NULL OR uid_next BETWEEN 0 AND 4294967295),
    highest_modseq INTEGER CHECK (highest_modseq IS NULL OR highest_modseq >= 0),
    selectable INTEGER NOT NULL CHECK (selectable IN (0, 1)),
    PRIMARY KEY (account_id, path)
) STRICT, WITHOUT ROWID;

CREATE TABLE mail_messages (
    account_id TEXT NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    folder TEXT NOT NULL CHECK (length(CAST(folder AS BLOB)) BETWEEN 1 AND 2048),
    uid INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    message_id TEXT CHECK (message_id IS NULL OR length(CAST(message_id AS BLOB)) <= 998),
    subject TEXT NOT NULL CHECK (length(CAST(subject AS BLOB)) <= 1024),
    from_json TEXT NOT NULL CHECK (length(CAST(from_json AS BLOB)) <= 65536 AND json_valid(from_json)),
    to_json TEXT NOT NULL CHECK (length(CAST(to_json AS BLOB)) <= 65536 AND json_valid(to_json)),
    cc_json TEXT NOT NULL CHECK (length(CAST(cc_json AS BLOB)) <= 65536 AND json_valid(cc_json)),
    date INTEGER,
    flags INTEGER NOT NULL CHECK (flags BETWEEN 0 AND 31),
    size INTEGER NOT NULL CHECK (size BETWEEN 0 AND 4294967295),
    has_attachments INTEGER NOT NULL CHECK (has_attachments IN (0, 1)),
    snippet TEXT NOT NULL CHECK (length(CAST(snippet AS BLOB)) <= 512),
    in_reply_to TEXT CHECK (in_reply_to IS NULL OR length(CAST(in_reply_to AS BLOB)) <= 998),
    references_json TEXT NOT NULL CHECK (length(CAST(references_json AS BLOB)) <= 65536 AND json_valid(references_json)),
    body_text TEXT CHECK (body_text IS NULL OR length(CAST(body_text AS BLOB)) <= 2097152),
    body_html TEXT CHECK (body_html IS NULL OR length(CAST(body_html AS BLOB)) <= 2097152),
    body_attachments_json TEXT CHECK (
        body_attachments_json IS NULL
        OR (length(CAST(body_attachments_json AS BLOB)) <= 262144 AND json_valid(body_attachments_json))
    ),
    body_truncated INTEGER NOT NULL DEFAULT 0 CHECK (body_truncated IN (0, 1)),
    body_bytes INTEGER NOT NULL DEFAULT 0 CHECK (body_bytes BETWEEN 0 AND 8388608),
    body_cached_at INTEGER,
    PRIMARY KEY (account_id, folder, uid)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_mail_messages_body_age ON mail_messages(body_cached_at) WHERE body_bytes > 0;
