ALTER TABLE sync_runtime_status
ADD COLUMN compacted_events INTEGER NOT NULL DEFAULT 0
CHECK (compacted_events >= 0);
