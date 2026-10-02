CREATE TABLE IF NOT EXISTS provider_ingestion (
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    PRIMARY KEY(namespace, name)
);
