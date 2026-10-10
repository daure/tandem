CREATE TABLE IF NOT EXISTS rule_instance_names (
    name_key TEXT PRIMARY KEY NOT NULL CHECK (
        length(name_key) BETWEEN 1 AND 40
        AND length(CAST(name_key AS BLOB)) = length(name_key)
        AND name_key NOT GLOB '*[^a-z0-9-]*'
        AND substr(name_key, 1, 1) GLOB '[a-z0-9]'
        AND name_key != 'gateway'
    ),
    namespace TEXT NOT NULL,
    acceptance_id INTEGER
);
CREATE TABLE IF NOT EXISTS rule_instance_name_backfill (
    id INTEGER PRIMARY KEY CHECK (id = 1)
);
CREATE INDEX IF NOT EXISTS runtime_records_name_key ON runtime_records(lower(name));
