CREATE TABLE sync_snapshot_publications (
    key_id             TEXT NOT NULL CHECK (LENGTH(key_id) = 64),
    creator_device_id  TEXT NOT NULL,
    state_hash         TEXT NOT NULL CHECK (LENGTH(state_hash) = 64),
    published_at       TEXT NOT NULL,

    PRIMARY KEY (key_id, creator_device_id)
);

CREATE TABLE sync_snapshot_imports (
    key_id             TEXT NOT NULL CHECK (LENGTH(key_id) = 64),
    creator_device_id  TEXT NOT NULL,
    state_hash         TEXT NOT NULL CHECK (LENGTH(state_hash) = 64),
    imported_at        TEXT NOT NULL,

    PRIMARY KEY (key_id, creator_device_id)
);
