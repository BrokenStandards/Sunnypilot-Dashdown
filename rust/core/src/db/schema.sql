-- v1 schema (applied by db::migrations). The mirror folder is the source of
-- truth; this DB is a rebuildable index. The `schema_version` table is created
-- by the migration runner, not here. Later milestones add tables via new
-- migrations (drive in M2, download_job in M4).

CREATE TABLE IF NOT EXISTS device (
    id                      INTEGER PRIMARY KEY,
    name                    TEXT    NOT NULL,
    dongle_label            TEXT,
    hotspot_ip              TEXT    NOT NULL,
    wifi_ip                 TEXT,
    port                    INTEGER NOT NULL,
    active_mode             TEXT    NOT NULL,   -- 'hotspot' | 'wifi'
    password                TEXT,
    auto_sync               INTEGER NOT NULL DEFAULT 0,
    file_selection          TEXT    NOT NULL DEFAULT 'previews_only',
    retention_max_minutes   INTEGER,
    auto_delete_from_comma  INTEGER NOT NULL DEFAULT 0,
    auto_delete_min_age_min INTEGER NOT NULL DEFAULT 60
);

CREATE TABLE IF NOT EXISTS segment (
    id          INTEGER PRIMARY KEY,
    device_id   INTEGER NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    route_id    TEXT    NOT NULL,
    segment_num INTEGER NOT NULL,
    recording   INTEGER NOT NULL DEFAULT 0,
    UNIQUE(device_id, route_id, segment_num)
);

CREATE TABLE IF NOT EXISTS seg_file (
    id             INTEGER PRIMARY KEY,
    segment_id     INTEGER NOT NULL REFERENCES segment(id) ON DELETE CASCADE,
    kind           TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    remote_size    INTEGER NOT NULL,
    mtime_s        INTEGER NOT NULL,
    local_size     INTEGER,                       -- populated by mirror/sync (M3+)
    download_state TEXT    NOT NULL DEFAULT 'missing',
    UNIQUE(segment_id, name)
);

CREATE INDEX IF NOT EXISTS idx_segment_device_route
    ON segment(device_id, route_id, segment_num);
