CREATE TABLE IF NOT EXISTS rule_named_sessions (
    namespace TEXT NOT NULL,
    instance TEXT NOT NULL,
    name TEXT NOT NULL,
    workspace_identity TEXT NOT NULL,
    session_id TEXT NOT NULL,
    server TEXT NOT NULL,
    ready INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (namespace, instance, name)
);
