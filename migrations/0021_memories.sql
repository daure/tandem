CREATE TABLE memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace TEXT NOT NULL,
    instance TEXT NOT NULL,
    workspace_identity TEXT NOT NULL,
    acceptance_id INTEGER,
    conclusion_id INTEGER UNIQUE,
    created_at TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    labels TEXT NOT NULL,
    links TEXT NOT NULL,
    markdown TEXT NOT NULL,
    fields TEXT NOT NULL
);
CREATE INDEX memories_namespace ON memories(namespace, id DESC);
CREATE INDEX memories_instance ON memories(namespace, instance, id DESC);
CREATE VIRTUAL TABLE memory_search USING fts5(
    title, labels, summary, markdown, fields, links,
    content='memories', content_rowid='id',
    tokenize='porter unicode61 remove_diacritics 2', prefix='2 3 4'
);
CREATE TRIGGER memories_insert AFTER INSERT ON memories BEGIN
    INSERT INTO memory_search(rowid, title, labels, summary, markdown, fields, links)
    VALUES (new.id, new.title, new.labels, new.summary, new.markdown, new.fields, new.links);
END;
CREATE TRIGGER memories_delete AFTER DELETE ON memories BEGIN
    INSERT INTO memory_search(memory_search, rowid, title, labels, summary, markdown, fields, links)
    VALUES ('delete', old.id, old.title, old.labels, old.summary, old.markdown, old.fields, old.links);
END;
INSERT INTO memories(namespace, instance, workspace_identity, acceptance_id, conclusion_id,
    created_at, created_ms, title, summary, labels, links, markdown, fields)
SELECT namespace, instance, workspace_identity, acceptance_id, id, admitted_at,
    CAST(unixepoch(admitted_at, 'subsec') * 1000 AS INTEGER),
    json_extract(report, '$.title'), json_extract(report, '$.summary'),
    COALESCE(json_extract(report, '$.labels'), '[]'), COALESCE(json_extract(report, '$.links'), '[]'),
    COALESCE(json_extract(report, '$.markdown'), ''), COALESCE(json_extract(report, '$.fields'), '{}')
FROM instance_conclusions WHERE report IS NOT NULL;
INSERT INTO memories(namespace, instance, workspace_identity, acceptance_id,
    created_at, created_ms, title, summary, labels, links, markdown, fields)
SELECT e.namespace, json_extract(a.payload, '$.instance'), '', a.id, r.reported_at,
    CAST(unixepoch(r.reported_at, 'subsec') * 1000 AS INTEGER),
    r.title, r.summary, '[]', '[]', r.markdown, '{}'
FROM rule_acceptance_reports r JOIN rule_acceptances a ON a.id = r.acceptance_id
JOIN event_attempts p ON p.id = a.attempt_id JOIN events e ON e.sequence = p.event_sequence
WHERE NOT EXISTS (SELECT 1 FROM memories m WHERE m.acceptance_id = a.id);
