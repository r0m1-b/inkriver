CREATE TABLE articles_with_extraction (
    id                          TEXT PRIMARY KEY NOT NULL,
    feed_id                     TEXT NOT NULL,
    title                       TEXT,
    author                      TEXT,
    published_at                TEXT,
    url                         TEXT,
    content                     TEXT,
    source                      TEXT NOT NULL CHECK (source IN ('medium', 'substack', 'other')),
    is_read                     INTEGER NOT NULL DEFAULT 0 CHECK (is_read IN (0, 1)),
    is_favorite                 INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
    content_kind                TEXT NOT NULL DEFAULT 'unknown'
                                CHECK (content_kind IN ('full', 'extracted', 'excerpt', 'missing', 'unknown')),
    is_archived                 INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
    archived_at                 TEXT,
    archive_reason              TEXT CHECK (archive_reason IS NULL OR archive_reason IN ('manual', 'retention')),
    extraction_attempted_at     TEXT,
    extraction_attempted_url    TEXT,
    extraction_attempt_count    INTEGER NOT NULL DEFAULT 0 CHECK (extraction_attempt_count >= 0),
    extraction_last_error       TEXT,

    FOREIGN KEY (feed_id) REFERENCES feeds(id) ON DELETE RESTRICT
);

INSERT INTO articles_with_extraction (
    id, feed_id, title, author, published_at, url, content, source,
    is_read, is_favorite, content_kind, is_archived, archived_at,
    archive_reason
)
SELECT
    id, feed_id, title, author, published_at, url, content, source,
    is_read, is_favorite, content_kind, is_archived, archived_at,
    archive_reason
FROM articles;

DROP TABLE articles;
ALTER TABLE articles_with_extraction RENAME TO articles;

CREATE INDEX articles_publication_date
    ON articles(published_at DESC);
