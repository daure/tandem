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

1. Press `T` or use the Template button to create a template named `website`; its folder contains a working static web starter.
2. On the template row, press `i` to inspect its directory, Compose file, and source; `d` copies the
   directory and `c` copies the Compose path inside that dialog.
3. Press `n`, enter `review`, then Enter or Ctrl+S to confirm execution; watch progress in Details.
4. Expand the template row using the DataView's configured expansion key (shown in its action bar)
   and select `review`; open `http://localhost:9876/review/web/index.html`.
5. Press `s` on an instance or service to confirm stopping it when running, or starting it when stopped.
   Service actions use existing containers and leave dependencies and the shared gateway untouched;
   workspace data and volumes are kept. The `.` Actions menu includes **Start service** and **Stop service**.
   Service start checks running state, configured healthchecks and gateway content within ten minutes;
   service stop has a one-minute budget. Each reports a completion or failure notification.
6. Press `r` on an instance or service for **Restart instance** or **Restart service**; confirm with `r`.
   Both actions are also in the `.` Actions menu. Restart uses existing containers, including stopped
   ones, preserves data and configuration, and skips one-shot setup jobs. A service restart leaves its
   dependencies untouched; the shared gateway is outside both scopes. A single completion notification
   appears after every restarted container is running, its configured healthcheck passes, and its
   configured gateway content assertion succeeds. Restart has a ten-minute budget; failures produce
   an error notification. Services without healthchecks or routes are verified only as running.
   Routed services require their current template readiness configuration. Applying template or image
   changes requires startup.

The TUI refreshes runtime inventory asynchronously every ten seconds while its terminal is focused,
every five minutes while unfocused, and immediately on regaining focus. Idle means terminal focus loss,
not time since the last keypress; terminals or multiplexers without focus reporting keep the ten-second
cadence. Local UI state updates every 250 ms.
Template files load at startup and on manual Refresh (`R`). The top-right refresh button refreshes
the full inventory; it shows `󰑓 Refresh` at 100 columns or wider and `󰑓 R` on narrower terminals,
with the displayed hotkey following configuration.
Manual refresh shows a completion notification after all requested checks and fresh resource samples
finish; failures appear as a warning. Automatic refreshes stay silent, and repeated manual
requests while one is pending share its completion notification.
Tandem mutations notify open TUIs through shared state, including MCP processes using the same
`TANDEM_HOME`. The observer checks small change counters every second and refreshes only affected
templates, runtime inventory, or settings after any in-flight refresh completes. External template
file edits require manual Refresh or reopening Tandem. Refresh reads state; it does not restart instances.
Resource rows show memory above CPU using Docker Engine one-shot samples through the selected local
Unix socket (API 1.41+). CPU is averaged between samples and displays `—` until two valid samples exist.
Sampling starts after container discovery, publishes memory, and takes a second reading after a
one-second pause to establish CPU usage. Automatic resource sampling runs alongside runtime refreshes
at one-minute intervals (normally every five minutes while unfocused); new or restarted containers
are sampled on discovery. Manual Refresh bypasses the interval, publishes memory immediately after
the first reading, and takes a second reading after one second for current CPU usage. Sampling requests
are serialized, including both readings.
An empty template list or unavailable Docker daemon is displayed without preventing template editing.
Starting an existing instance name reapplies the same template directory. It refuses names owned by
another template or unmanaged containers.

## Agent workflow

```json
{"command":"tandem","args":["mcp"]}
```

The MCP tools are `get_instructions`, `list_templates`, `get_template`, `create_template`, `update_template_manifest`,
`list_instances`, `create_instance`, `get_operation`, `stop_instance`, `start_service`, `stop_service`, `restart_instance`, `restart_service`, `get_open_command`,
`set_open_command`, `run_open_command`, and `get_status`.
List tools return objects with `templates` or `instances` arrays. Mutating instance tools require
`confirmed=true` after user approval; create waits for readiness by default, or accepts `wait=false`.
`get_operation` reports in-process progress, elapsed time, and `running`/`succeeded`/`failed` outcomes.
Use runtime-derived `list_instances` after reconnecting to a different MCP process.

[agent-instructions.md](agent-instructions.md) contains lightweight guidance on Tandem's intended use.
On first launch it seeds `$TANDEM_HOME/instructions.md`, which `get_instructions` reads on every call.
The response includes that editable path, the active template/workspace roots, and `manifest_schema`,
a JSON Schema generated from Tandem's Rust manifest types. The editable guidance file is preserved.
To edit the repository
Markdown directly while running:

```bash
TANDEM_INSTRUCTIONS_FILE="$PWD/agent-instructions.md" cargo run -- dev
```

`opencode.json` points to the development HTTP MCP server; start `cargo run -- dev` before connecting.

Use `update_template_manifest` with `name`, a complete `manifest` JSON object, and `confirmed=true`
after approval to replace shared template configuration. It rejects unknown fields and invalid
manifest rules before atomically saving a private `tandem.json` under the template lock; omitted
optional fields take their defaults. The full replacement must fit within the 256 KiB manifest limit.
It can repair invalid JSON and create a missing manifest in an existing template. Symlinked targets
are refused. Saving does not run Docker or Git, restart instances, or validate Compose service
references; those checks occur during startup. Direct file edits are validated when templates are read.

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
  "repositories": [{"source": "https://github.com/team/app.git", "target": "app"}],
  "routes": {
    "web": {"port": 80, "strip_prefix": true, "readiness_path": "index.html", "readiness_contains": "My application"}
  },
  "one_shots": []
}
```

Route and one-shot names must match Compose service names. Readiness paths are relative to the
service's public URL, and content assertions must be nonempty. Omit one-shots if there are none.
Configure application base paths, asset links, redirects, authentication callbacks, and hot-reload
WebSockets for the chosen prefix. A proxy cannot make every application prefix-aware automatically.

The template receives `TANDEM_INSTANCE`, `TANDEM_WORKSPACE`, `TANDEM_ORIGIN`, `TANDEM_UID`, and
`TANDEM_GID`. When the local Branch instances setting is enabled, new instances also receive
`TANDEM_BRANCH` set to their instance name. Put writable source checkouts and per-instance data under
the workspace; template files are shared. Installed dependency trees and databases must be instance-specific.

### Repository provisioning

Declare `{ "source": "…", "target": "…" }` entries in `repositories` to let Tandem prepare checkouts
before Compose configuration and startup. Mount `${TANDEM_WORKSPACE}` into application containers;
build contexts and includes can reference its declared repository paths. Templates own application
setup and migrations, with dependencies gated on their one-shot completion.

Sources accept absolute local paths, HTTPS URLs without embedded credentials, and `ssh://` URLs.
Use host Git credential helpers or SSH agents; SSH requires an existing known-host entry and runs
without prompts. Native provisioning requires Linux and Git 2.23+; it uses the startup deadline,
instance lock and host user. It does not execute repository hooks or recursively initialize submodules.
Targets are non-overlapping relative paths of non-hidden directories using letters, digits, dots,
underscores and hyphens. Symlinked paths, unrelated contents and mismatched origins are rejected.

For missing checkouts, Branch instances selects a matching remote branch or creates a local branch
from the remote default when missing. With the setting disabled, the default branch is used.
A local source's current branch is its default. Tandem never pushes these branches.
Each clone is installed atomically after branch selection. Existing checkouts must have the exact
declared origin and a real `.git` directory; they retain their current branch and all local work,
even when the branch setting changes. Restarting performs no fetch, pull, reset or branch switch.

Templates without `repositories` use their existing checkout scripts. To migrate, declare the same
sources and targets, remove clone services such as `repo-sync` and their dependencies, and retain
app-specific setup jobs. Do not run two provisioners against the same checkout. Existing runtime
guidance files are user-owned; update them explicitly when adopting this workflow.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `TANDEM_HOME` | platform data directory / `tandem` | Absolute local root for templates, workspaces, locks, gateway, and instructions |
| `TANDEM_NAMESPACE` | `tandem` | Docker project/network namespace |
| `TANDEM_GATEWAY_PORT` | `9876` | Shared loopback browser port |
| `TANDEM_INSTRUCTIONS_FILE` | `$TANDEM_HOME/instructions.md` | Existing Markdown file, read on every tool call |
| `TANDEM_KEY_INFO` | `i` | Template information dialog |
| `TANDEM_KEY_START` | `n` | Start instance dialog |
| `TANDEM_KEY_NEW_TEMPLATE` | `T` | Create template dialog |
| `TANDEM_KEY_STOP` | `s` | Start/stop the selected instance or service; stop all instances on a template row |
| `TANDEM_KEY_REFRESH` | `R` | Refresh template/runtime inventory |
| `TANDEM_KEY_DELETE` | `x` | Delete the selected instance, or a template with all its instances and data |
| `TANDEM_KEY_PURGE` | `p` | Purge all instances of the selected template |
| `TANDEM_KEY_RESTART` | `r` | Restart the selected instance or service after confirmation |

Application hotkey overrides are distinct ASCII letters. Shared navigation, focus, and component
keys use tuicore configuration. The TUI displays resolved key labels.

### Workspace open command

In Settings, edits to **Open command** save immediately. Enter on an instance opens its workspace
with `xdg-open` when the setting is empty or whitespace-only.
A custom command runs on the host via `sh -c`, with the workspace as its working directory, the
instance name in `TANDEM_INSTANCE`, and the absolute path in `TANDEM_WORKSPACE`. Quote it as `"$TANDEM_WORKSPACE"`; paths are passed as
environment data, not interpolated into shell code. Enter on a routed service opens its URL.

The setting is stored in `$TANDEM_HOME/settings.sqlite3`. MCP agents can read it with
`get_open_command` and save it with `set_open_command` using `command` and `confirmed=true`
after approval. Saving never executes the command. Opening an instance reads the current persisted
value, including changes from another process; Tandem saves notify active TUI Settings caches.
Close and reopen Settings to display external changes.

`run_open_command` executes the persisted command for a named instance workspace after user approval
with `confirmed=true`. It accepts stopped instances whose workspace remains available.

Commands inherit the TUI process's environment, including terminal-multiplexer session variables,
but have disconnected stdin/stdout/stderr. Use noninteractive launcher commands; failures are
recorded in Tandem's diagnostic logs. Commands run asynchronously and do not block keyboard input.

## Safety and lifecycle

- Only run trusted templates: Compose and scripts have local Docker/host privileges.
- Only the gateway publishes host ports, bound to `127.0.0.1`; its Docker socket access is privileged.
- Instances share browser-origin state and the ingress network; these are not security boundaries.
- Readiness requires configured response content, not just HTTP 200. Missing healthchecks are `up`;
  successful one-shot completion is `exited 0`. Pending compilation and route registration are normal.
- Instance and gateway mutations use advisory locks shared by processes using the same Tandem home.
- Delete template permanently removes its instances, private volumes and networks, workspace folders,
  and template files after confirmation. Shared Docker images, external resources, and the gateway
  remain outside its ownership scope. Ownership receipts and verified rendered Compose files identify
  leftover instance data; cleanup failures retain the template for a retry.
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
