CREATE TABLE IF NOT EXISTS event_rules (
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    revision INTEGER NOT NULL,
    definition TEXT NOT NULL,
    zellij_session TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    catalog_present INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY(namespace, name)
);
CREATE TABLE IF NOT EXISTS rule_evaluations (
    attempt_id INTEGER NOT NULL REFERENCES event_attempts(id),
    rule_name TEXT NOT NULL,
    rule_revision INTEGER NOT NULL,
    rule_snapshot TEXT NOT NULL,
    outcome TEXT,
    error TEXT,
    PRIMARY KEY(attempt_id, rule_name)
);
CREATE TABLE IF NOT EXISTS rule_acceptances (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_id INTEGER NOT NULL REFERENCES event_attempts(id),
    rule_name TEXT NOT NULL,
    payload TEXT NOT NULL,
    UNIQUE(attempt_id, rule_name)
);
CREATE INDEX IF NOT EXISTS rule_acceptance_history ON rule_acceptances(rule_name, id DESC);
CREATE TABLE IF NOT EXISTS rule_dispatch_starts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    acceptance_id INTEGER NOT NULL REFERENCES rule_acceptances(id),
    started_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS rule_dispatch_rate ON rule_dispatch_starts(started_at);
