CREATE TABLE rule_dispatch_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    acceptance_id INTEGER NOT NULL,
    started_at TEXT NOT NULL
);
INSERT INTO rule_dispatch_history(id, namespace, acceptance_id, started_at)
SELECT s.id, e.namespace, s.acceptance_id, s.started_at
FROM rule_dispatch_starts s
JOIN rule_acceptances a ON a.id = s.acceptance_id
JOIN event_attempts p ON p.id = a.attempt_id
JOIN events e ON e.sequence = p.event_sequence;
DROP TABLE rule_dispatch_starts;
ALTER TABLE rule_dispatch_history RENAME TO rule_dispatch_starts;
CREATE INDEX rule_dispatch_rate ON rule_dispatch_starts(namespace, started_at);
