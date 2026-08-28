CREATE TABLE sync_runtime_status (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_attempt_at TEXT,
    last_success_at TEXT,
    last_error_stage TEXT,
    last_error_message TEXT,
    last_error_at TEXT,
    uploaded_segments INTEGER NOT NULL DEFAULT 0 CHECK (uploaded_segments >= 0),
    reused_segments INTEGER NOT NULL DEFAULT 0 CHECK (reused_segments >= 0),
    exported_events INTEGER NOT NULL DEFAULT 0 CHECK (exported_events >= 0),
    downloaded_segments INTEGER NOT NULL DEFAULT 0 CHECK (downloaded_segments >= 0),
    received_events INTEGER NOT NULL DEFAULT 0 CHECK (received_events >= 0),
    imported_events INTEGER NOT NULL DEFAULT 0 CHECK (imported_events >= 0),
    duplicate_events INTEGER NOT NULL DEFAULT 0 CHECK (duplicate_events >= 0),
    applied_events INTEGER NOT NULL DEFAULT 0 CHECK (applied_events >= 0),
    pending_events INTEGER NOT NULL DEFAULT 0 CHECK (pending_events >= 0),
    CHECK (
        (last_error_stage IS NULL AND last_error_message IS NULL AND last_error_at IS NULL)
        OR
        (last_error_stage IS NOT NULL AND last_error_message IS NOT NULL AND last_error_at IS NOT NULL)
    )
);
