# Tandem contribution guide

1. Read `architecture.md` and `~/dev/tuicore/SKILL.md` before changing application code.
2. Keep `AppService` as the sole boundary for domain operations, persistence, and external clients. TUI nodes and MCP tools call the service; they do not implement domain rules or I/O.
3. Use `tuicore` for terminal lifecycle, semantic themes, focus, layouts, and configurable keybindings. Render paths are pure.
4. Keep MCP stdio protocol data on stdout only. Diagnostics belong in Tandem's state-directory logs or stderr.
5. Put tests in sibling `tests/` directories. Treat warnings as build failures.
6. Maintain `agent-instructions.md` according to the guidance below.

## Local state publications

`serve-state` exposes private Unix-socket NDJSON through `AppService`, independently of the TUI.
CLI listings and stream objects share domain projections. Publish completed, allowlisted state;
do not collect new data per subscriber or forward raw OpenCode payloads, task input, or credentials.
`store/state_stream.rs` owns versioned envelopes and entity comparisons; `environments/state_stream`
owns the bounded journal, atomic snapshot/cursor boundary, origin associations, and shared observation
leases. New observable domain state belongs in this publication path, with its event and replay
semantics documented beside the CLI protocol. Consumers tolerate unknown event types.

Keep origin binding ahead of first publication, preserve uncertainty across worker/response loss,
and enforce deletion markers inside every session-write transaction. Replay describes recorded
publications; it cannot promise unobserved external history. Keep queues, writes, journal retention,
and shared collection bounded. Blocking publishers retain their observation lease through commit.
Test reconnect, origin races, stale reads, and slow clients when changing this contract.

## OpenCode live verification

After changing `src/environments/opencode/bridge.mjs`, install the rebuilt companion at the path
used by `~/.config/opencode/cli.json` (V1: `tui.json`):

```bash
cargo build --bin tandem
TANDEM_HOME="$HOME/.local/share/tandem" target/debug/tandem opencode-setup
```

Do not use `cargo run` for this installation because the repository runner sources
`projects/env.sh` and targets the development Tandem home. Verify the installed `bridge.mjs`
contains the change, then tell the user it was installed and that open clients need restarting.

For changes to the OpenCode companion, session tracking, or Zellij navigation, offer this
development-only check and obtain user approval before running it:

```bash
python3 src/environments/opencode/tests/live.py
```

Requires Linux with Python 3, OpenCode (verified with 1.18.29 and 2.0.22), and Zellij 0.45 or later on `PATH`.
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

The instance-scoped MCP uses `instance-agent-instructions.md` as its editable guidance seed and
`instance-core-guidance.md` as its bundled baseline. Keep that guidance focused on scoped lifecycle,
accepted-task reporting, preservation before purge, and historical-report reuse.

- Keep it concise and self-contained. Consolidate related guidance when adding a rule; never refer readers to other documents, source-checkout guides, or external resources.
- Include only facts that change an MCP agent's decisions. Distinguish Tandem guarantees from template conventions.
- MCP schemas own tool parameters; the README owns TUI usage; this file and `architecture.md` own contributor guidance.
- Keep UI details, implementation internals, changelogs, and duplicated API reference out of runtime guidance.
- Verify claims against code and reread for redundancy before saving. Update it only when agent-facing workflows change.
