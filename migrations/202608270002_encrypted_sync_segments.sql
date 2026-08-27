CREATE TABLE sync_export_cursors (
    key_id TEXT PRIMARY KEY NOT NULL,
    last_exported_sequence INTEGER NOT NULL DEFAULT 0
        CHECK (last_exported_sequence >= 0)
);
