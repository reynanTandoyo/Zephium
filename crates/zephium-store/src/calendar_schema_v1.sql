-- Fork (T-0041): local calendar cache. Application-wide, in the meta database.
-- No secret lives here: CalDAV passwords are only in the OS vault. Row counts
-- are held by the writes; the raw iCalendar text is bounded here.

CREATE TABLE calendar_accounts (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(CAST(id AS BLOB)) BETWEEN 1 AND 64 AND id NOT GLOB '*[^A-Za-z0-9]*'
    ),
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('caldav', 'subscription')),
    name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) <= 1024),
    server_url TEXT NOT NULL CHECK (length(CAST(server_url AS BLOB)) BETWEEN 1 AND 8192),
    username TEXT CHECK (username IS NULL OR length(CAST(username AS BLOB)) <= 1024)
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

CREATE INDEX idx_calendar_events_range ON calendar_events(calendar_id, start_utc);
CREATE INDEX idx_calendar_events_uid ON calendar_events(calendar_id, uid);
