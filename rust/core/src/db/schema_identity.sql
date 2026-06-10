-- v4 schema (B1): per-device transport identity, kept out of the `device`
-- record (and thus the FFI surface) so it's internal sync-engine state. Pins the
-- copyparty server hostname (the stable "same device" anchor) + the last leaf
-- TLS fingerprint (tolerated to rotate when the hostname matches), and caches
-- the last base URL (scheme://ip:port) that worked so the resolver tries it
-- first instead of re-probing every IP/scheme.

CREATE TABLE IF NOT EXISTS device_identity (
    device_id      INTEGER PRIMARY KEY REFERENCES device(id) ON DELETE CASCADE,
    hostname       TEXT,
    cert_sha256    TEXT,
    last_good_base TEXT
);
