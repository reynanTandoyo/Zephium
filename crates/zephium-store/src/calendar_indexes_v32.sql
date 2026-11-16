-- Fork (review N4): v31 rebuilt the calendar tables and recreated only
-- idx_calendars_account; restore the two event indexes of v30.
CREATE INDEX IF NOT EXISTS idx_calendar_events_range ON calendar_events(calendar_id, start_utc);
CREATE INDEX IF NOT EXISTS idx_calendar_events_uid ON calendar_events(calendar_id, uid);
