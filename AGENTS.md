# Tandem contribution guide

1. Read `architecture.md` and `~/dev/tuicore/SKILL.md` before changing application code.
2. Keep `AppService` as the sole boundary for domain operations, persistence, and external clients. TUI nodes and MCP tools call the service; they do not implement domain rules or I/O.
3. Use `tuicore` for terminal lifecycle, semantic themes, focus, layouts, and configurable keybindings. Render paths are pure.
4. Keep MCP stdio protocol data on stdout only. Diagnostics belong in Tandem's state-directory logs or stderr.
5. Put tests in sibling `tests/` directories. Treat warnings as build failures.
6. Keep `agent-instructions.md` up to date as Tandem's behavior and workflows evolve. Keep it lightweight and focused on how Tandem works, intended usage, and deeper agent guidance; MCP tool schemas own the API reference, so do not duplicate tool documentation there.
