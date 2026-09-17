CREATE TABLE IF NOT EXISTS instance_startups (
  id INTEGER PRIMARY KEY,
  template TEXT NOT NULL,
  duration_milliseconds INTEGER NOT NULL CHECK (duration_milliseconds >= 0)
);
