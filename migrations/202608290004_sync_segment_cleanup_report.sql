ALTER TABLE sync_runtime_status
ADD COLUMN deleted_segments INTEGER NOT NULL DEFAULT 0
CHECK (deleted_segments >= 0);

ALTER TABLE sync_runtime_status
ADD COLUMN deferred_segment_deletions INTEGER NOT NULL DEFAULT 0
CHECK (deferred_segment_deletions >= 0);
