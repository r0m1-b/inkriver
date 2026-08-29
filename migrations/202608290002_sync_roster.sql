CREATE TABLE sync_roster_members (
    key_id           TEXT NOT NULL CHECK (LENGTH(key_id) = 64),
    device_id        TEXT NOT NULL,
    revoked_at       TEXT,
    first_observed_at TEXT NOT NULL,
    last_observed_at  TEXT NOT NULL,

    PRIMARY KEY (key_id, device_id)
);
