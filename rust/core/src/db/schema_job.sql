-- v3 schema (M4): one row per drive download, persisting progress + terminal
-- state so the UI/native scheduler can report results across a process kill (the
-- Android Foreground Service / iOS BGTask may be terminated mid-download). The
-- mirror folder remains the source of truth for what is actually on disk; this
-- row is advisory and rebuildable. Resume/recovery from a stale 'running' row
-- after a crash is M5.

CREATE TABLE IF NOT EXISTS download_job (
    id          INTEGER PRIMARY KEY,
    device_id   INTEGER NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    drive_key   TEXT    NOT NULL,
    state       TEXT    NOT NULL DEFAULT 'running', -- running | complete | failed | canceled
    files_total INTEGER NOT NULL DEFAULT 0,
    files_done  INTEGER NOT NULL DEFAULT 0,
    bytes_total INTEGER NOT NULL DEFAULT 0,
    bytes_done  INTEGER NOT NULL DEFAULT 0,
    error       TEXT,
    updated_s   INTEGER NOT NULL DEFAULT 0,         -- epoch seconds (std::time::SystemTime)
    UNIQUE(device_id, drive_key)
);
