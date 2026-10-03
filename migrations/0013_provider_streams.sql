CREATE TABLE IF NOT EXISTS provider_streams (
    namespace TEXT NOT NULL,
    provider TEXT NOT NULL,
    stream TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    active INTEGER NOT NULL DEFAULT 1,
    revision INTEGER NOT NULL DEFAULT 1,
    applied_revision INTEGER,
    applied_at INTEGER,
    requested_at INTEGER,
    error TEXT,
    PRIMARY KEY (namespace, provider, stream)
);
