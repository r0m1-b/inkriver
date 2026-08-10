ALTER TABLE articles
ADD COLUMN content_kind TEXT NOT NULL DEFAULT 'unknown'
CHECK (content_kind IN ('full', 'excerpt', 'missing', 'unknown'));

UPDATE articles
SET content_kind = 'missing'
WHERE content IS NULL;
