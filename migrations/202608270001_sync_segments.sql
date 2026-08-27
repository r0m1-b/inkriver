ALTER TABLE sync_local_state
ADD COLUMN last_exported_sequence INTEGER NOT NULL DEFAULT 0
CHECK (last_exported_sequence >= 0 AND last_exported_sequence < next_sequence);
