CREATE TABLE IF NOT EXISTS rule_acceptance_workspaces (
    acceptance_id INTEGER PRIMARY KEY REFERENCES rule_acceptances(id) ON DELETE CASCADE,
    payload TEXT NOT NULL
);
