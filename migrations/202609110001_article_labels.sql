CREATE TABLE labels (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL
                    CHECK (LENGTH(TRIM(name)) BETWEEN 1 AND 80),
    normalized_name TEXT NOT NULL UNIQUE
                    CHECK (LENGTH(TRIM(normalized_name)) BETWEEN 1 AND 160)
);

CREATE TABLE article_labels (
    article_id TEXT NOT NULL,
    label_id   TEXT NOT NULL,

    PRIMARY KEY (article_id, label_id),
    FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE,
    FOREIGN KEY (label_id) REFERENCES labels(id) ON DELETE CASCADE
);

CREATE INDEX article_labels_by_label ON article_labels(label_id, article_id);
