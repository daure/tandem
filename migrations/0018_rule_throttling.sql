ALTER TABLE rule_evaluations ADD COLUMN deadline_ms INTEGER;
ALTER TABLE rule_evaluations ADD COLUMN warning TEXT;
CREATE INDEX rule_evaluations_due ON rule_evaluations(deadline_ms) WHERE outcome = 'deferred';

CREATE TABLE IF NOT EXISTS rule_throttles (
    namespace TEXT NOT NULL,
    rule_name TEXT NOT NULL,
    until_ms INTEGER NOT NULL,
    PRIMARY KEY (namespace, rule_name)
);
