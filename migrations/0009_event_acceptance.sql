CREATE TABLE event_attempts_accepted (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_sequence INTEGER NOT NULL REFERENCES events(sequence),
    status TEXT NOT NULL CHECK(status IN ('pending', 'accepted')),
    replay INTEGER NOT NULL,
    request_id TEXT,
    created_at TEXT NOT NULL,
    accepted_at TEXT,
    UNIQUE(event_sequence, request_id)
);
INSERT INTO event_attempts_accepted
    (id, event_sequence, status, replay, request_id, created_at, accepted_at)
SELECT id, event_sequence,
    CASE status WHEN 'handled' THEN 'accepted' ELSE status END,
    replay, request_id, created_at, handled_at
FROM event_attempts;
UPDATE sqlite_sequence SET seq = MAX(seq, COALESCE(
    (SELECT seq FROM sqlite_sequence WHERE name = 'event_attempts'), 0))
WHERE name = 'event_attempts_accepted';
DROP TABLE event_attempts;
ALTER TABLE event_attempts_accepted RENAME TO event_attempts;
CREATE INDEX event_attempt_history ON event_attempts(event_sequence, id DESC);
