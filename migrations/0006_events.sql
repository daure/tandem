CREATE TABLE IF NOT EXISTS event_providers (
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    PRIMARY KEY (namespace, name)
);
CREATE TABLE IF NOT EXISTS events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    provider TEXT NOT NULL,
    event_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    received_at TEXT NOT NULL,
    UNIQUE (namespace, provider, event_id),
    FOREIGN KEY (namespace, provider) REFERENCES event_providers(namespace, name)
);
CREATE INDEX IF NOT EXISTS events_feed ON events(namespace, sequence DESC);
CREATE TABLE IF NOT EXISTS event_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_sequence INTEGER NOT NULL REFERENCES events(sequence),
    status TEXT NOT NULL CHECK(status IN ('pending', 'handled')),
    replay INTEGER NOT NULL,
    request_id TEXT,
    created_at TEXT NOT NULL,
    handled_at TEXT,
    UNIQUE(event_sequence, request_id)
);
CREATE INDEX IF NOT EXISTS event_attempt_history ON event_attempts(event_sequence, id DESC);
CREATE TABLE IF NOT EXISTS provider_notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    provider TEXT NOT NULL,
    payload TEXT NOT NULL,
    acknowledged INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(namespace, provider) REFERENCES event_providers(namespace, name)
);
CREATE INDEX IF NOT EXISTS provider_notification_queue
    ON provider_notifications(namespace, provider, acknowledged, id);
