-- v2 schema (M2): the `drive` index. A drive is a maximal run of consecutive
-- segments within one route (see drive_grouping::group_segments). Drives are
-- rebuildable from `segment` by re-grouping; `preserved` and `sync_state` carry
-- user/sync intent that must survive a regroup, so `replace_drives` upserts the
-- derived columns and leaves those two untouched (behavior lands in M6 / M5).

CREATE TABLE IF NOT EXISTS drive (
    id            INTEGER PRIMARY KEY,
    device_id     INTEGER NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    drive_key     TEXT    NOT NULL,            -- first segment dir_name
    route_id      TEXT    NOT NULL,
    first_seg     INTEGER NOT NULL,
    last_seg      INTEGER NOT NULL,
    start_ms      INTEGER,                      -- NULL when the first segment has no files
    end_ms        INTEGER,                      -- NULL when the last segment has no files
    segment_count INTEGER NOT NULL,
    recording     INTEGER NOT NULL DEFAULT 0,
    preserved     INTEGER NOT NULL DEFAULT 0,   -- user pin; behavior: M6
    sync_state    TEXT    NOT NULL DEFAULT 'not_downloaded', -- behavior: M5
    UNIQUE(device_id, drive_key)
);

CREATE INDEX IF NOT EXISTS idx_drive_device ON drive(device_id, route_id, first_seg);
