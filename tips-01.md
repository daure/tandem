# Authoring template-specific agent guidance

Suggested addition to `agent-instructions.md`, after the existing `tandem-agents.md` bullet:

Write `tandem-agents.md` for agents developing applications in the template’s environment. Include template-specific facts that affect their work: reload/rebuild behavior, test commands and execution locations, useful logs or data inspection points, and operational hazards. Put any project-specific verification expectations here; generated workspace guidance describes available capabilities without prescribing a verification workflow. Keep guidance concise and reusable across instances: reference generated service mappings and URLs rather than hardcoding instance names, ports, or container IDs. Avoid duplicating repository instructions, and verify commands and paths against the template before documenting them.
