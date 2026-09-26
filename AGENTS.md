# Tandem contribution guide

1. Read `architecture.md` and `~/dev/tuicore/SKILL.md` before changing application code.
2. Keep `AppService` as the sole boundary for domain operations, persistence, and external clients. TUI nodes and MCP tools call the service; they do not implement domain rules or I/O.
3. Use `tuicore` for terminal lifecycle, semantic themes, focus, layouts, and configurable keybindings. Render paths are pure.
4. Keep MCP stdio protocol data on stdout only. Diagnostics belong in Tandem's state-directory logs or stderr.
5. Put tests in sibling `tests/` directories. Treat warnings as build failures.
6. Maintain `agent-instructions.md` according to the guidance below.

## OpenCode live verification

For changes to the OpenCode companion, session tracking, or Zellij navigation, offer this
development-only check and obtain user approval before running it:

```bash
python3 src/environments/opencode/tests/live.py
```

Requires Linux with Python 3, OpenCode (verified with 1.18.29), and Zellij 0.45 or later on `PATH`.
It starts temporary servers and terminal clients with isolated configuration, sends no model
prompts, and cleans up its test resources. Passing checks cover companion loading, conversation
switches, exact stacked-pane focus, and client-close detection.

Keep this check explicitly opt-in and outside routine test commands, CI, builds, installation,
and release workflows. Python is a development prerequisite for this check, not a Tandem runtime
dependency.

## MCP agent guidance

`agent-instructions.md` is the seed for the editable runtime guidance served by `get_instructions`.
It teaches agents using Tandem how to configure templates, manage repository/workspace workflows,
and operate environments safely. Existing runtime copies are user-owned and preserved.

- Keep it concise and self-contained. Consolidate related guidance when adding a rule; never refer readers to other documents, source-checkout guides, or external resources.
- Include only facts that change an MCP agent's decisions. Distinguish Tandem guarantees from template conventions.
- MCP schemas own tool parameters; the README owns TUI usage; this file and `architecture.md` own contributor guidance.
- Keep UI details, implementation internals, changelogs, and duplicated API reference out of runtime guidance.
- Verify claims against code and reread for redundancy before saving. Update it only when agent-facing workflows change.
