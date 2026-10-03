UPDATE event_rules
SET definition = json_remove(definition, '$.metadata')
WHERE json_type(definition, '$.metadata') IS NOT NULL;

UPDATE rule_evaluations
SET rule_snapshot = json_remove(rule_snapshot, '$.definition.metadata')
WHERE json_type(rule_snapshot, '$.definition.metadata') IS NOT NULL;

UPDATE rule_acceptances
SET payload = json_remove(payload, '$.rule.definition.metadata')
WHERE json_type(payload, '$.rule.definition.metadata') IS NOT NULL;
