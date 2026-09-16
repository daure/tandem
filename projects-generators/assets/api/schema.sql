CREATE TABLE IF NOT EXISTS greetings (
    id integer PRIMARY KEY CHECK (id = 1),
    message text NOT NULL
);
INSERT INTO greetings (id, message) VALUES (1, 'Hello, world!')
ON CONFLICT (id) DO NOTHING;
