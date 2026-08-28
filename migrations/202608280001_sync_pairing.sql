CREATE TABLE sync_configuration (
    singleton        INTEGER PRIMARY KEY NOT NULL DEFAULT 1 CHECK (singleton = 1),
    webdav_base_url  TEXT NOT NULL,
    webdav_username  TEXT NOT NULL,
    key_id           TEXT NOT NULL CHECK (LENGTH(key_id) = 64)
);

CREATE TABLE sync_devices (
    device_id        TEXT PRIMARY KEY NOT NULL,
    display_name     TEXT NOT NULL CHECK (
                         LENGTH(TRIM(display_name)) BETWEEN 1 AND 120
                         AND display_name = TRIM(display_name)
                     ),
    is_local         INTEGER NOT NULL DEFAULT 0 CHECK (is_local IN (0, 1)),
    revoked_at       TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

CREATE UNIQUE INDEX sync_devices_single_local
    ON sync_devices(is_local)
    WHERE is_local = 1;
