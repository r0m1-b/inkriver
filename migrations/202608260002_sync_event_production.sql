ALTER TABLE sync_local_state
ADD COLUMN is_enabled INTEGER NOT NULL DEFAULT 0
CHECK (is_enabled IN (0, 1));
