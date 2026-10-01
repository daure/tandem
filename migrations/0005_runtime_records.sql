CREATE TABLE IF NOT EXISTS runtime_records (
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('journal', 'startup', 'ownership', 'launch')),
    payload TEXT NOT NULL,
    PRIMARY KEY (namespace, name, kind)
);
CREATE TABLE IF NOT EXISTS runtime_imports (
    namespace TEXT PRIMARY KEY,
    imported_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS runtime_import_files (
    namespace TEXT NOT NULL,
    path TEXT NOT NULL,
    original TEXT NOT NULL,
    PRIMARY KEY (namespace, path)
);
CREATE TABLE IF NOT EXISTS refresh_revisions (
    scope TEXT PRIMARY KEY,
    revision INTEGER NOT NULL
);
