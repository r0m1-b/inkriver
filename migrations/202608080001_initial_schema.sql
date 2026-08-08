CREATE TABLE feeds (
    id          TEXT PRIMARY KEY NOT NULL,
    platform    TEXT NOT NULL CHECK (platform IN ('medium', 'substack', 'other')),
    url         TEXT NOT NULL,
    is_active   INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1))
);

CREATE UNIQUE INDEX feeds_unique_active_url
    ON feeds(url)
    WHERE is_active = 1;

CREATE TABLE articles (
    id              TEXT PRIMARY KEY NOT NULL,
    feed_id         TEXT NOT NULL,
    title           TEXT,
    author          TEXT,
    published_at    TEXT,
    url             TEXT,
    content         TEXT,
    source          TEXT NOT NULL CHECK (source IN ('medium', 'substack', 'other')),
    is_read         INTEGER NOT NULL DEFAULT 0 CHECK (is_read IN (0, 1)),
    is_favorite     INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),

    FOREIGN KEY (feed_id) REFERENCES feeds(id) ON DELETE RESTRICT
);

CREATE INDEX articles_publication_date
    ON articles(published_at DESC);
