ALTER TABLE articles ADD COLUMN entry_key TEXT;

UPDATE articles
SET entry_key = CASE
    WHEN SUBSTR(id, 1, LENGTH(feed_id) + 2) = feed_id || '::'
        THEN SUBSTR(id, LENGTH(feed_id) + 3)
    ELSE id
END;

CREATE UNIQUE INDEX articles_feed_entry_key
    ON articles(feed_id, entry_key)
    WHERE entry_key IS NOT NULL;

CREATE TABLE sync_local_state (
    singleton             INTEGER PRIMARY KEY NOT NULL DEFAULT 1
                          CHECK (singleton = 1),
    device_id             TEXT NOT NULL UNIQUE,
    next_sequence         INTEGER NOT NULL DEFAULT 1
                          CHECK (next_sequence > 0),
    hlc_physical_ms       INTEGER NOT NULL DEFAULT 0
                          CHECK (hlc_physical_ms >= 0),
    hlc_counter           INTEGER NOT NULL DEFAULT 0
                          CHECK (hlc_counter >= 0)
);

CREATE TRIGGER sync_local_device_id_immutable
BEFORE UPDATE OF device_id ON sync_local_state
BEGIN
    SELECT RAISE(ABORT, 'sync device identity is immutable');
END;

CREATE TABLE sync_events (
    device_id             TEXT NOT NULL,
    sequence              INTEGER NOT NULL CHECK (sequence > 0),
    hlc_physical_ms       INTEGER NOT NULL CHECK (hlc_physical_ms >= 0),
    hlc_counter           INTEGER NOT NULL CHECK (hlc_counter >= 0),
    protocol_version      INTEGER NOT NULL CHECK (protocol_version > 0),
    event_kind            TEXT NOT NULL CHECK (
                              LENGTH(event_kind) BETWEEN 1 AND 64
                              AND event_kind = TRIM(event_kind)
                          ),
    payload_json          TEXT NOT NULL,

    PRIMARY KEY (device_id, sequence)
);

CREATE INDEX sync_events_version
    ON sync_events(hlc_physical_ms, hlc_counter, device_id, sequence);

CREATE TRIGGER sync_events_immutable
BEFORE UPDATE ON sync_events
BEGIN
    SELECT RAISE(ABORT, 'sync events are immutable');
END;

CREATE TABLE sync_import_cursors (
    remote_device_id      TEXT PRIMARY KEY NOT NULL,
    contiguous_sequence   INTEGER NOT NULL DEFAULT 0
                          CHECK (contiguous_sequence >= 0)
);

CREATE TABLE sync_pending_events (
    device_id             TEXT NOT NULL,
    sequence              INTEGER NOT NULL,
    reason                TEXT NOT NULL CHECK (LENGTH(TRIM(reason)) > 0),

    PRIMARY KEY (device_id, sequence),
    FOREIGN KEY (device_id, sequence)
        REFERENCES sync_events(device_id, sequence) ON DELETE CASCADE
);

CREATE TABLE sync_subscription_aliases (
    alias_id                       TEXT PRIMARY KEY NOT NULL,
    canonical_id                   TEXT NOT NULL,
    normalized_url                 TEXT NOT NULL,
    parent_tombstone_device_id     TEXT,
    parent_tombstone_sequence      INTEGER,

    CHECK (
        (parent_tombstone_device_id IS NULL AND parent_tombstone_sequence IS NULL)
        OR
        (parent_tombstone_device_id IS NOT NULL AND parent_tombstone_sequence IS NOT NULL)
    )
);

CREATE INDEX sync_subscription_incarnations
    ON sync_subscription_aliases(
        normalized_url,
        parent_tombstone_device_id,
        parent_tombstone_sequence
    );

CREATE TABLE sync_entity_versions (
    entity_kind           TEXT NOT NULL
                          CHECK (entity_kind IN ('subscription', 'article')),
    entity_key            TEXT NOT NULL,
    field_name            TEXT NOT NULL CHECK (LENGTH(TRIM(field_name)) > 0),
    event_device_id       TEXT NOT NULL,
    event_sequence        INTEGER NOT NULL,

    PRIMARY KEY (entity_kind, entity_key, field_name),
    FOREIGN KEY (event_device_id, event_sequence)
        REFERENCES sync_events(device_id, sequence) ON DELETE RESTRICT
);

CREATE TABLE sync_tombstones (
    entity_kind           TEXT NOT NULL
                          CHECK (entity_kind IN ('subscription', 'article')),
    entity_key            TEXT NOT NULL,
    event_device_id       TEXT NOT NULL,
    event_sequence        INTEGER NOT NULL,

    PRIMARY KEY (entity_kind, entity_key),
    FOREIGN KEY (event_device_id, event_sequence)
        REFERENCES sync_events(device_id, sequence) ON DELETE RESTRICT
);

CREATE TABLE sync_article_identities (
    subscription_id       TEXT NOT NULL,
    entry_key             TEXT NOT NULL,
    article_id            TEXT UNIQUE,

    PRIMARY KEY (subscription_id, entry_key),
    FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE SET NULL
);
