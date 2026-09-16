# Tandem

Keyboard-first local development environments, shared by a TUI and MCP agents. Templates are editable
Compose directories; each instance gets its own Compose project and workspace. A label-driven Traefik
gateway exposes declared HTTP services at `http://localhost:9876/<instance>/<service>/`.

## Quick start

Requires a local Docker Engine, Docker Compose v2 or newer, and Rust. The source checkout uses the
sibling `../tuicore` package. Container images are pulled on first use.

```bash
cargo run                   # TUI
cargo run -- dev            # TUI + HTTP MCP at http://127.0.0.1:7348/mcp
cargo run -- mcp            # protocol-only stdio MCP
cargo run -- serve          # HTTP MCP at http://127.0.0.1:7345/mcp
```

1. Press `t` to create a template named `website`; its folder contains a working static web starter.
2. On the template row, press `i` to inspect its directory, Compose file, and source; `d` copies the
   directory and `c` copies the Compose path inside that dialog.
3. Press `n`, enter `review`, then Enter or Ctrl+S to confirm execution; watch progress in Details.
4. Expand the template row using the DataView's configured expansion key (shown in its action bar)
   and select `review`; open `http://localhost:9876/review/web/index.html`.
5. Press `s` on an instance to confirm stopping its containers; the workspace and volumes are kept.

The TUI refreshes runtime inventory asynchronously every two seconds and local progress every 250 ms.
An empty template list or unavailable Docker daemon is displayed without preventing template editing.
Starting an existing instance name reapplies the same template directory. It refuses names owned by
another template or unmanaged containers.

## Agent workflow

```json
{"command":"tandem","args":["mcp"]}
```

The MCP tools are `get_instructions`, `list_templates`, `get_template`, `create_template`,
`list_instances`, `create_instance`, `get_operation`, `stop_instance`, and `get_status`.
List tools return objects with `templates` or `instances` arrays. Mutating instance tools require
`confirmed=true` after user approval; create waits for readiness by default, or accepts `wait=false`.
`get_operation` reports in-process progress, elapsed time, and `running`/`succeeded`/`failed` outcomes.
Use runtime-derived `list_instances` after reconnecting to a different MCP process.

[agent-instructions.md](agent-instructions.md) contains lightweight guidance on Tandem's intended use.
On first launch it seeds `$TANDEM_HOME/instructions.md`, which `get_instructions` reads on every call.
The response includes that editable path and the active template/workspace roots. To edit the repository
Markdown directly while running:

```bash
TANDEM_INSTRUCTIONS_FILE="$PWD/agent-instructions.md" cargo run -- dev
```

`opencode.json` points to the development HTTP MCP server; start `cargo run -- dev` before connecting.

## Template layout and routing

```text
<TANDEM_HOME>/templates/website/
  compose.yaml
  tandem.json
  site/index.html
  scripts/                       # optional, edited by the agent
  config/                        # optional, edited by the agent
```

Relative mounts, builds, env files, and scripts resolve against the template directory. Tandem renders
a private `.tandem-<namespace>-<instance>.compose.json` beside the Compose file. The rendered artifact
may contain secrets; keep it out of Git. The starter includes an appropriate `.gitignore`.

`tandem.json` declares each public service's internal port, prefix behavior, and readiness content
assertion. No route is inferred for databases or other non-HTTP services. `strip_prefix=true` sends
`/<instance>/<service>/index.html` upstream as `/index.html`; `false` preserves the full path.
For example:

```json
{
  "description": "Local application",
  "routes": {
    "web": {"port": 80, "strip_prefix": true, "readiness_path": "index.html", "readiness_contains": "My application"}
  },
  "one_shots": ["repo-sync"]
}
```

Route and one-shot names must match Compose service names. Readiness paths are relative to the
service's public URL, and content assertions must be nonempty. Omit one-shots if there are none.
Configure application base paths, asset links, redirects, authentication callbacks, and hot-reload
WebSockets for the chosen prefix. A proxy cannot make every application prefix-aware automatically.

The template receives `TANDEM_INSTANCE`, `TANDEM_WORKSPACE`, `TANDEM_ORIGIN`, `TANDEM_UID`, and
`TANDEM_GID`. Put writable source checkouts and per-instance data under the workspace; template files
are shared. Clone/sync belongs in a template one-shot service, with dependent services gated on its
successful completion. Installed dependency trees and databases must be instance-specific.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `TANDEM_HOME` | platform data directory / `tandem` | Absolute local root for templates, workspaces, locks, gateway, and instructions |
| `TANDEM_NAMESPACE` | `tandem` | Docker project/network namespace |
| `TANDEM_GATEWAY_PORT` | `9876` | Shared loopback browser port |
| `TANDEM_INSTRUCTIONS_FILE` | `$TANDEM_HOME/instructions.md` | Existing Markdown file, read on every tool call |
| `TANDEM_KEY_INFO` | `i` | Template information dialog |
| `TANDEM_KEY_START` | `n` | Start instance dialog |
| `TANDEM_KEY_NEW_TEMPLATE` | `t` | Create template dialog |
| `TANDEM_KEY_STOP` | `s` | Stop instance confirmation |
| `TANDEM_KEY_REFRESH` | `r` | Refresh template/runtime inventory |

Application hotkey overrides are distinct lowercase letters. Shared navigation, focus, and component
keys use tuicore configuration. The TUI displays resolved key labels.

## Safety and lifecycle

- Only run trusted templates: Compose and scripts have local Docker/host privileges.
- Only the gateway publishes host ports, bound to `127.0.0.1`; its Docker socket access is privileged.
- Instances share browser-origin state and the ingress network; these are not security boundaries.
- Readiness requires configured response content, not just HTTP 200. Missing healthchecks are `up`;
  successful one-shot completion is `exited 0`. Pending compilation and route registration are normal.
- Instance and gateway mutations use advisory locks shared by processes using the same Tandem home.
- Failed/timed-out creation preserves resources and workspaces for diagnosis. Retry after inspecting
  the error and Docker logs. There is no cancellation or destructive workspace-prune tool.
- Stopping removes containers and private networks; shared gateway, workspaces, and volumes remain.
  Docker labels are the instance source of truth; there is no persistent instance registry.

To shut down the shared gateway deliberately (this interrupts **all** instance URLs):

```bash
docker compose -p tandem-gateway -f "${TANDEM_HOME:-$HOME/.local/share/tandem}/gateway/compose.json" down
```

Adjust the project and path for a custom namespace/data directory. MCP HTTP is loopback-only and
unauthenticated; it accepts loopback Host headers and same-origin browser requests. Add an authenticated
transport before remote use. Diagnostics go to state-directory
logs or stderr; MCP stdout contains protocol data only.

## Verification

For editable sample applications, use the [development fixture generators](projects-generators/README.md).
They create five local Git repositories and four Tandem templates under ignored `projects/`.
On Unix, `cargo run -- dev` sources the checkout's `projects/env.sh` when present, before launching
Tandem. Its exports override inherited values for that process; your current shell stays unchanged.
The file is trusted shell code. Other commands use their inherited environment; source the generated
file manually to use fixtures with plain `cargo run` or MCP-only mode.

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
# Exercise the actual binary and stdio protocol, including a separate observer process:
cargo build
PYTHONDONTWRITEBYTECODE=1 python3 -W error -m unittest discover -s src/mcp/tests
python3 src/mcp/tests/stdio_smoke.py --live
# Exercise isolated fixture instances, routing, Git edits, persistence, and failure modes:
PYTHONDONTWRITEBYTECODE=1 python3 -W error projects-generators/tests/live_smoke.py
# Exercise the complete keyboard workflow in a pseudo-terminal:
uv run --with pyte --with pexpect python src/app/tests/pty_smoke.py
```

## Release

From `main` with committed source changes, a sibling `../tuicore` checkout, Git push access, Python 3.11+, Rust, and an authenticated GitHub CLI (`gh auth login`):

```bash
cargo release          # patch: 0.1.0 -> 0.1.1
cargo release minor    # minor: 0.1.0 -> 0.2.0
cargo release major   # major: 0.1.0 -> 1.0.0
# Equivalent command without compiling the tiny Cargo helper:
./scripts/release.sh patch
```

The command checks the branch and local Tuicore path, then runs the release workflow's formatting, script tests, Clippy, and Rust tests before changing the version. It then bumps Tandem, refreshes and commits the local lockfile, creates an annotated tag without opening an editor or pager, and atomically pushes `main` and its `vX.Y.Z` tag. Generated `Cargo.lock` changes are accepted; all other files must be clean. It returns without waiting for GitHub Actions. Existing `cargo patch`, `cargo minor`, and `cargo major` aliases use the same release flow.

GitHub Actions resolves the highest stable, non-yanked Tuicore version from crates.io before running Cargo checks and builds. This only modifies the disposable CI checkout; local development always builds the sibling `../tuicore` checkout. Registry or compatibility failures stop the build.
