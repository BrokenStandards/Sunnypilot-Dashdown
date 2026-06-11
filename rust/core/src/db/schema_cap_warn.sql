-- v5: per-device controls for the low-headroom storage warning. Default ON with a
-- 10-minute threshold so existing devices keep the prior always-on behavior.
ALTER TABLE device ADD COLUMN cap_warn_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE device ADD COLUMN cap_warn_threshold_minutes INTEGER NOT NULL DEFAULT 10;
