CREATE TABLE IF NOT EXISTS provider_deletions (
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    PRIMARY KEY (namespace, name),
    UNIQUE (namespace, source)
);
