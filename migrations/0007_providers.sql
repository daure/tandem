CREATE TABLE IF NOT EXISTS provider_launches (
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    directory TEXT NOT NULL,
    manifest TEXT NOT NULL,
    compose TEXT NOT NULL,
    error TEXT,
    PRIMARY KEY(namespace, name)
);
