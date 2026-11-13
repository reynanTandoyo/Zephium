-- Fork (T-0040): calendar accounts that sign in with OAuth (Google, Microsoft).
-- The account kind gains 'oauth'; the provider and the person's own client ID
-- and tenant (not secrets) get their own columns. Refresh tokens and client
-- secrets are only in the OS vault. SQLite cannot change a CHECK in place, so
-- the three calendar tables are rebuilt with their rows kept: the children
-- are dropped before their parent, so no cascade can remove a row.

CREATE TEMP TABLE backup_calendar_accounts AS
    SELECT id, position, kind, name, server_url, username FROM calendar_accounts;
CREATE TEMP TABLE backup_calendars AS
    SELECT id, account_id, position, href, display_name, colour, read_only, visible,
           ctag, sync_token, synced_at FROM calendars;
CREATE TEMP TABLE backup_calendar_events AS
    SELECT calendar_id, href, etag, uid, ical, start_utc, end_utc, last_seen
    FROM calendar_events;

DROP TABLE calendar_events;
DROP TABLE calendars;
DROP TABLE calendar_accounts;

CREATE TABLE calendar_accounts (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(CAST(id AS BLOB)) BETWEEN 1 AND 64 AND id NOT GLOB '*[^A-Za-z0-9]*'
    ),
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('caldav', 'subscription', 'oauth')),
    name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) <= 1024),
    server_url TEXT NOT NULL CHECK (length(CAST(server_url AS BLOB)) BETWEEN 1 AND 8192),
    username TEXT CHECK (username IS NULL OR length(CAST(username AS BLOB)) <= 1024),
    oauth_provider TEXT CHECK (oauth_provider IS NULL OR oauth_provider IN ('google', 'microsoft')),
    oauth_client_id TEXT CHECK (
        oauth_client_id IS NULL OR length(CAST(oauth_client_id AS BLOB)) BETWEEN 1 AND 200
    ),
    oauth_tenant TEXT CHECK (
        oauth_tenant IS NULL OR length(CAST(oauth_tenant AS BLOB)) BETWEEN 1 AND 128
    ),
    CHECK (
        (kind = 'oauth' AND oauth_provider IS NOT NULL AND oauth_client_id IS NOT NULL)
        OR (kind <> 'oauth' AND oauth_provider IS NULL AND oauth_client_id IS NULL
            AND oauth_tenant IS NULL)
    )
) STRICT;

CREATE TABLE calendars (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(CAST(id AS BLOB)) BETWEEN 1 AND 64 AND id NOT GLOB '*[^A-Za-z0-9]*'
    ),
    account_id TEXT NOT NULL REFERENCES calendar_accounts(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    href TEXT NOT NULL CHECK (length(CAST(href AS BLOB)) BETWEEN 1 AND 8192),
    display_name TEXT NOT NULL CHECK (length(CAST(display_name AS BLOB)) <= 1024),
    colour TEXT CHECK (colour IS NULL OR length(colour) = 7),
    read_only INTEGER NOT NULL CHECK (read_only IN (0, 1)),
    visible INTEGER NOT NULL CHECK (visible IN (0, 1)),
    ctag TEXT CHECK (ctag IS NULL OR length(CAST(ctag AS BLOB)) <= 4096),
    sync_token TEXT CHECK (sync_token IS NULL OR length(CAST(sync_token AS BLOB)) <= 4096),
    synced_at INTEGER
) STRICT;

CREATE INDEX idx_calendars_account ON calendars(account_id);

CREATE TABLE calendar_events (
    calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    href TEXT NOT NULL CHECK (length(CAST(href AS BLOB)) BETWEEN 1 AND 8192),
    etag TEXT CHECK (etag IS NULL OR length(CAST(etag AS BLOB)) <= 1024),
    uid TEXT NOT NULL CHECK (length(CAST(uid AS BLOB)) <= 1024),
    ical TEXT NOT NULL CHECK (length(CAST(ical AS BLOB)) <= 8388608),
    start_utc INTEGER NOT NULL,
    end_utc INTEGER,
    last_seen INTEGER NOT NULL,
    PRIMARY KEY (calendar_id, href)
) STRICT, WITHOUT ROWID;

INSERT INTO calendar_accounts(id, position, kind, name, server_url, username)
    SELECT id, position, kind, name, server_url, username FROM backup_calendar_accounts;
INSERT INTO calendars(id, account_id, position, href, display_name, colour, read_only,
                      visible, ctag, sync_token, synced_at)
    SELECT id, account_id, position, href, display_name, colour, read_only, visible,
           ctag, sync_token, synced_at FROM backup_calendars;
INSERT INTO calendar_events(calendar_id, href, etag, uid, ical, start_utc, end_utc, last_seen)
    SELECT calendar_id, href, etag, uid, ical, start_utc, end_utc, last_seen
    FROM backup_calendar_events;

DROP TABLE backup_calendar_events;
DROP TABLE backup_calendars;
DROP TABLE backup_calendar_accounts;
