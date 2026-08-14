ALTER TABLE articles
ADD COLUMN is_archived INTEGER NOT NULL DEFAULT 0
CHECK (is_archived IN (0, 1));

ALTER TABLE articles
ADD COLUMN archived_at TEXT;

ALTER TABLE articles
ADD COLUMN archive_reason TEXT
CHECK (archive_reason IS NULL OR archive_reason IN ('manual', 'retention'));
