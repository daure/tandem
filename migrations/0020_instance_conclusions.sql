CREATE TABLE IF NOT EXISTS instance_conclusions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    instance TEXT NOT NULL,
    workspace_identity TEXT NOT NULL,
    acceptance_id INTEGER REFERENCES rule_acceptances(id) ON DELETE CASCADE,
    report TEXT,
    admitted_at TEXT NOT NULL,
    cleanup_state TEXT NOT NULL CHECK (cleanup_state IN ('pending', 'purging', 'purged', 'failed')),
    cleanup_error TEXT,
    UNIQUE(namespace, instance, workspace_identity)
);
CREATE INDEX IF NOT EXISTS instance_conclusions_acceptance ON instance_conclusions(acceptance_id);
