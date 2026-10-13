CREATE TABLE IF NOT EXISTS state_stream_heads (
    namespace TEXT PRIMARY KEY,
    epoch TEXT NOT NULL,
    sequence INTEGER NOT NULL DEFAULT 0,
    floor INTEGER NOT NULL DEFAULT 0,
    bytes INTEGER NOT NULL DEFAULT 0,
    snapshot TEXT NOT NULL,
    observation TEXT
);
CREATE TABLE IF NOT EXISTS state_stream_events (
    namespace TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY(namespace, sequence)
);
CREATE TABLE IF NOT EXISTS session_origins (
    namespace TEXT NOT NULL,
    server TEXT NOT NULL,
    session_id TEXT NOT NULL,
    origin TEXT NOT NULL,
    directory TEXT NOT NULL,
    session TEXT NOT NULL,
    PRIMARY KEY(namespace, server, session_id, origin)
);
CREATE TABLE IF NOT EXISTS session_creation_requests (
    namespace TEXT NOT NULL,
    id TEXT NOT NULL,
    directory TEXT NOT NULL,
    instance TEXT NOT NULL,
    origin TEXT,
    server TEXT,
    session_id TEXT,
    observed INTEGER NOT NULL DEFAULT 0,
    completed INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(namespace, id)
);
CREATE TABLE IF NOT EXISTS state_observations (
    namespace TEXT NOT NULL,
    kind TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    payload TEXT NOT NULL,
    PRIMARY KEY(namespace, kind)
);
CREATE TABLE IF NOT EXISTS state_session_edits (
    namespace TEXT NOT NULL,
    server TEXT NOT NULL,
    session_id TEXT NOT NULL,
    title TEXT,
    PRIMARY KEY(namespace, server, session_id)
);
