CREATE TABLE IF NOT EXISTS rule_acceptance_reports (
    acceptance_id INTEGER PRIMARY KEY REFERENCES rule_acceptances(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    markdown TEXT NOT NULL,
    reported_at TEXT NOT NULL,
    cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('pending', 'purging', 'purged', 'failed')),
    cleanup_error TEXT
);
