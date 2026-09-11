CREATE TABLE categories (
    id   TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE
         CHECK (LENGTH(TRIM(name)) BETWEEN 1 AND 80)
);

ALTER TABLE feeds
ADD COLUMN category_id TEXT REFERENCES categories(id) ON DELETE SET NULL;

CREATE INDEX feeds_category_id ON feeds(category_id);
