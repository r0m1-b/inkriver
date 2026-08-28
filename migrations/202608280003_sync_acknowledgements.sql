CREATE TABLE sync_acknowledgements (
    key_id                TEXT NOT NULL CHECK (LENGTH(key_id) = 64),
    observer_device_id    TEXT NOT NULL,
    source_device_id      TEXT NOT NULL,
    contiguous_sequence   INTEGER NOT NULL DEFAULT 0
                          CHECK (contiguous_sequence >= 0),
    observed_at           TEXT NOT NULL,

    PRIMARY KEY (key_id, observer_device_id, source_device_id)
);

CREATE INDEX sync_acknowledgements_source
    ON sync_acknowledgements(key_id, source_device_id, contiguous_sequence);
