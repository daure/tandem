# Tandem

Keyboard-first local development environments, shared by a TUI, CLI and MCP agents. Templates are editable
development recipes; each instance gets its own workspace and, when services are configured, a Compose project. A label-driven Traefik
gateway exposes declared HTTP services at `http://localhost:9876/<instance>/<service>/`.

## Quick start

Building requires Rust and the sibling `../tuicore` package. Service templates require a local Docker
Engine and Docker Compose v2 or newer; container images are pulled on first use. Repository-only
templates require Linux and host Git but their lifecycle does not require Docker.
Guidance-only templates prepare a workspace folder without Git or Docker.

```bash
cargo run                   # TUI
cargo run -- dev            # TUI + HTTP MCP at http://127.0.0.1:7348/mcp; replaces a prior dev server using this Tandem home and port
cargo run -- mcp            # protocol-only stdio MCP
cargo run -- serve          # HTTP MCP at http://127.0.0.1:7345/mcp
```

1. Press `T` or use the Template button to create a template named `website`; its folder contains a working static web starter.
2. Press `Enter` on a template, instance or service to view its details. Template details include
   metadata, available Compose source and the manifest.
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
Template files load at startup and on manual Refresh (`R`). The toolbar refresh button refreshes
the full inventory; it shows `󰑓 Refresh` at 100 columns or wider and `󰑓 R` on narrower terminals,
with the displayed hotkey following configuration.
Beside Refresh, ** Stop all** and ** Purge all** act across every template, including instances whose
template files are missing. Their hotkeys are `S` and `P`; below 100 columns they show icons and hotkey
badges. Both require confirmation for the
targets captured when the dialog opens; instances created afterward are outside that confirmation.
Stop all preserves data and is disabled when no instance can be stopped. Purge all permanently removes
instance containers, workspaces, volumes and networks and is disabled when there are no instances.
Both preserve templates and the shared gateway. Each instance reports its own outcome; a failure does
not prevent the other instances from being attempted.
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
are serialized, including both readings. Creating/starting instances show `—` until their operation
ends; a failed startup still permits metrics for surviving running containers. Paused containers retain
memory readings with CPU `— · paused`. Collapse, filtering and scrolling leave sampling/totals intact.
Partial totals disclose missing coverage; stale readings retain their age and lose pressure coloring.
Memory pressure is amber at 70% and red at 90% of explicit caps; aggregate coloring requires complete,
fresh, capped coverage. Resource errors do not change runtime health.
An empty template list or unavailable Docker daemon is displayed without preventing template editing.
Starting an existing instance name reapplies the same template directory. It refuses names owned by
another template or unmanaged containers.

## CLI instance lifecycle

```bash
tandem new-instance review --template website
tandem new-instance review -t website --open-command
tandem new-instance review -t website -oc
tandem new-instance review -t website -oc "project notes"
tandem delete-instance review
tandem delete-instance review --close-command
tandem delete-instance review -cc --headless
```

For a new instance, the template must exist. Invoking this command authorizes host Git provisioning
and Docker execution; use trusted templates. Creation follows the saved Branch instances setting and
the same ownership checks, locks and startup rules as the TUI/MCP. The CLI waits up to ten minutes for
startup readiness, prints the workspace and service URLs on success, and exits nonzero on failure.
Failed startups preserve resources for inspection.

An existing instance owned by the requested template is left unchanged, including when stopped or
paused. The CLI reports that it exists without checking readiness. With `--open-command` or `-oc`,
it only launches the saved opener in that instance's workspace; template files and repository sources
are not needed. Ownership mismatches and concurrent instance operations are rejected.

`--open-command` (also spelled `-oc`) optionally accepts an open parameter. `--description` (also spelled `-d`)
sets the instance description. For a new instance, the workspace-ready signal
triggers the saved opener after declared repositories have been cloned or validated, any Compose configuration
has been rendered, and the workspace `AGENTS.md` has been written. Opening and container startup then
proceed independently. Template-owned
clone scripts and container-created files are outside this milestone; declare repositories for early
source access.
An empty saved command uses the system folder opener. Without the flag, nothing is opened.
The opener inherits the CLI's environment; custom commands run in the workspace with `TANDEM_INSTANCE`,
`TANDEM_WORKSPACE`, and `TANDEM_DESCRIPTION` set. When `--open-command` has an open parameter, it is
available as `TANDEM_OPEN_PARAM`.
Editor lifetime does not delay startup or CLI exit. Settings/launch failures
make the CLI fail after startup completes, or immediately for an existing instance. Opener exit failures
observed while the CLI runs are logged.

`delete-instance` permanently removes the named instance's owned containers, workspace, private volumes,
networks, rendered Compose file, and ownership receipt. It leaves templates, shared images, and the gateway intact.
`--close-command` (also `-cc`) opts into running the saved close command before workspace removal;
without it, CLI deletion skips that command. An empty saved command does nothing.
`--headless` (also `-h`) launches deletion in a detached Tandem process and returns after that process starts.
It preserves the close-command flag but cannot report the deletion result; inspect diagnostic logs or
runtime state for failures.

### Workspace agent context

[workspace-agents.template.md](workspace-agents.template.md) is the default workspace guidance template.
On launch, Tandem installs the bundled template at `$TANDEM_HOME/workspace-agents.template.md`.
When its bundled content changes, the first launch after installation/update refreshes that shared
template before creating instances; this also applies to `cargo run`. A differing local copy is saved
alongside it as `workspace-agents.template.<unique>.bak` before replacement. Local edits survive
restarts while the bundled content stays the same. Existing workspace `AGENTS.md` files are preserved.
`get_instructions` returns the editable template's absolute path as `workspace_agents_template`.

Template authors can place optional `tandem-agents.md` beside `compose.yaml` and `tandem.json`.
When present, it appears as a **Guidance** tab in template details, with Markdown syntax
highlighting. Its contents are appended verbatim after the rendered workspace guidance, separated by
a blank line; `{{...}}` expressions in this file stay literal. It accepts UTF-8 text up to 256 KiB and
must resolve to a regular file within the template directory. Read errors block generation.
Template inspection exposes its absolute `guidance_file` path and optional `guidance_source` content.
Refresh templates after editing; changes apply to future generation, while existing workspace files
remain user-owned.

The template supports `{{instance}}`, `{{template}}`, `{{project}}`, `{{repositories}}`,
`{{repository_guidance}}`, `{{compose_project_command}}`, `{{docker_discovery_command}}`,
`{{http_urls}}`, `{{http_guidance}}`, `{{http_section}}`, `{{services}}`, `{{compose_command}}`,
`{{workspace_description}}`, and `{{inventory_heading}}`.
Conditional sections use `{{#inventory}}…{{/inventory}}` (resource inventory or unknown container configuration),
`{{#services}}…{{/services}}`, `{{#repositories}}…{{/repositories}}`,
`{{#code_paths}}…{{/code_paths}}` (repositories and services), and `{{#local_only}}…{{/local_only}}`
(repositories without services). Values are literal substitutions; unknown, unclosed or mismatched
expressions and empty output fail generation. `{{repositories}}` renders an inventory table, covering every configured
service except `repo-sync`, plus declared targets and top-level Git checkouts (including worktrees).
Repository rows include container code paths and root `AGENTS.md` or `agents.md` paths (`None` when
absent). Each service/path mapping gets its own row. Services without an identified repository mapping
use `—` in the repository, code-path and guidance columns. Unmapped repositories remain listed. The repository
guidance instruction appears only when those files are listed. Default guidance uses the instance name
as its heading and includes repository instructions, Compose access and gateway HTTP URLs only when
those resources are identified. Repository-only guidance uses a repository/guidance table and local
verification instructions. Guidance-only workspaces contain an instance heading, a short local-work
introduction, and the template's literal guidance. `{{http_guidance}}` and `{{http_section}}` provide the URL-testing sentence and
complete HTTP URLs section only when a service has a generated URL. Template-local `tandem-agents.md`
is appended verbatim; its author owns any route-specific guidance.
`{{compose_project_command}}` supplies `docker compose -p` with the quoted project;
`{{compose_command}}` also includes the rendered configuration path and project directory. Custom templates
can include service entries with names, roles, images, gateway URLs and configured route ports,
without environment values or live metrics.

Services and repository mappings use the instance's ownership-verified rendered Compose file, including
infrastructure, setup jobs and services with zero replicas. When that file is missing, known instance
service names are listed once each. Direct repository
binds identify code paths; whole-workspace binds also need a matching working directory, command or
entrypoint argument, or local repository build context. Paths can represent a mounted repository
subdirectory and can differ from the container's working directory. Missing configuration or
unidentified mappings display `Not identified`; named volumes and image-only code require runtime
inspection. Malformed or mismatched rendered configuration blocks generation.

Every new instance receives a root `AGENTS.md` after repository preparation and before any container startup, even without an open command.
CLI, TUI and MCP opening also generate it if missing; generation failures block the opener, including
the system folder opener. Existing regular files are preserved, while symlinks and directories are
rejected. This is a configuration snapshot: keep local guidance current as repositories or services
change. To regenerate from the editable template, move the workspace file aside and open the instance.
Template-owned clone jobs and container-created files appear only after their own setup completes.

## Statuses

Services show their runtime/health status; instance summaries include running/expected counts and
the reason for degradation. Running without a healthcheck is blue **Running**; green **Healthy**
requires Docker healthchecks. Gateway readiness is a separate timestamped check in Details.
Intentional stops show **Stopped**, including signal exits such as 137/143; raw exit codes alone
do not prove failure. **Paused** is distinct, and setup jobs use **Completed**, **Failed** or
**Interrupted**. The collapsed **Setup** group includes successfully provisioned Git repositories,
sorted by target path above completed one-shot jobs. Repository rows occupy one line, start with a
success-colored ``, and show **Cloned** or **Existing checkout** from the latest preparation.
Press `yy` on a repository row to copy its absolute checkout directory, also available in Actions.
The completion count includes repositories and completed jobs; provisioning results survive reconnects.
Activities use tuicore's spinner;
names stay neutral while status labels and issue qualifiers carry semantic colors.
Tandem retains launch topology and run-specific stop evidence across processes. Instances without
recorded topology show **Unknown** until an approved Start captures their configuration.
Deletion remains visible through cleanup, with failed cleanup retained as an operation row.
Select a failed instance-cleanup row and press `x`, or choose **Retry cleanup** in Actions, to retry
with the deletion confirmation. If containers are absent, recovery requires a matching runtime record
and template ownership receipt; it checks Docker project membership before removing remaining data.
Recovery with missing execution-kind metadata requires Docker access. Unverifiable ownership leaves data intact.

Workspace-only instances (repository-only or guidance-only) show **Workspace ready** after preparation and guidance generation, retain
their workspace path, and show a single muted **(no services configured)** child when expanded. They hide
CPU/memory metrics and disable container Start/Stop/Restart actions; Open, Details and Delete remain
available. Failed preparation retains completed checkouts and reports its error; retry New instance
with the same template/name after correcting the cause. Docker failures do not make these workspaces stale.

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
Instance listings also include `activities` for active work and retained failures, including cleanup
without containers; per-instance/service `summary` and `runtime` separate interpretation from evidence.
Workspace-only instances come from durable workspace records. If Docker inspection fails while these
records exist, `list_instances` returns them with `runtime_error`; container inventory is unavailable.

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

A template may contain `compose.yaml`, a repository-declaring `tandem.json`, `tandem-agents.md`, or a combination. Compose-only
templates use empty manifest defaults: services have no Tandem routes, one-shot roles, or managed clones.
Repository-only templates omit `compose.yaml`, declare at least one repository, and omit routes and
one-shots. Guidance-only templates supply `tandem-agents.md` with an optional manifest and prepare an
otherwise empty workspace containing generated `AGENTS.md`. Templates without Compose require either
repositories or a guidance file; empty unrelated directories are not templates. An empty Compose services
map is invalid. The template creator supplies a Compose starter;
edit the dedicated template directory to select another shape. Existing instances retain their launch
kind; use a new instance name to switch between workspace-only and container-backed execution.

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
| `TANDEM_KEY_INFO` | `Enter` | View details for the selected template, instance or service |
| `TANDEM_KEY_START` | `n` | Start instance dialog |
| `TANDEM_KEY_NEW_TEMPLATE` | `T` | Create template dialog |
| `TANDEM_KEY_STOP` | `s` | Start/stop the selected instance or service; stop all instances on a template row |
| `TANDEM_KEY_REFRESH` | `R` | Refresh template/runtime inventory |
| `TANDEM_KEY_DELETE` | `x` | Delete the selected template with all its instances and data |
| `TANDEM_KEY_PURGE` | `p` | Purge the selected instance, or all instances of the selected template |
| `TANDEM_KEY_RESTART` | `r` | Restart the selected instance or service after confirmation |
| `TANDEM_KEY_STOP_ALL` | `S` | Stop instances across all templates after confirmation |
| `TANDEM_KEY_PURGE_ALL` | `P` | Purge instances across all templates after confirmation |
| `TANDEM_KEY_OPEN_COMMAND` | `ctrl+;` | Run the saved open command for the selected instance |

Application hotkey overrides accept distinct ASCII letters; `TANDEM_KEY_INFO` also accepts `Enter`,
and `TANDEM_KEY_OPEN_COMMAND` also accepts `ctrl+;`. Shared navigation, focus, and component keys use
tuicore configuration. The TUI displays resolved key labels. Ctrl+; requires a terminal that reports
the modifier; the Actions menu or a letter override works when the terminal cannot send it.

### Workspace commands

In Settings, edits to **Open command** save immediately. Press `Ctrl+;` on an instance or choose
**Run open command** from its `.` Actions menu to execute it. An empty or whitespace-only setting
opens the workspace with `xdg-open`.
Press `y`, or choose **Yank** from the `.` Actions menu, to open the copy menu for the selected
instance or routed service. For an instance, `i` copies its name and `w` copies its absolute workspace path;
for a routed service, `u` copies its URL.
A custom command runs on the host via `sh -c`, with the workspace as its working directory, the
instance name in `TANDEM_INSTANCE`, the absolute path in `TANDEM_WORKSPACE`, and its description in
`TANDEM_DESCRIPTION`. Quote `"$TANDEM_WORKSPACE"`; paths are passed as
environment data, not interpolated into shell code. Choose **Open in browser** from a routed service's
`.` Actions menu, or press `Ctrl+Enter` on that service, to open its URL. On an instance,
`Ctrl+Enter` opens its only routed service directly or displays a chooser when several are available;
each chooser option is shown as `<service> - <url>`.

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

**Close command** in Settings saves immediately. TUI and MCP deletion run it automatically before
removing an existing instance workspace, after its Docker resources are removed; CLI deletion requires
`--close-command` or `-cc`. It uses host `sh -c`, the
workspace as its working directory, and the same `TANDEM_INSTANCE` and `TANDEM_WORKSPACE` variables.
An empty or whitespace-only command disables the hook. Individual deletion, Purge all, template
purges, and template deletion use it; stop and restart preserve workspaces and do not run it.
The command has a ten-second limit. Launch failures, nonzero exits, and timeouts produce warnings
while deletion continues. Warnings appear in TUI completion notifications, CLI stderr, MCP operation
`warnings`, and diagnostic logs. Cleanup remains subject to its operation deadline.
Missing workspaces skip the hook; unsafe workspace paths fail validation before command execution.
Use repeat-safe commands because a failed filesystem removal can cause the hook to run again on retry.

MCP agents read and save this shared setting through `get_close_command` and `set_close_command`;
saving requires approval with `confirmed=true` and does not execute it. Deletion reads the persisted
value, including changes from another process. Configure only trusted host commands.

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
On Unix, `cargo run` sources the checkout's `projects/env.sh` when present for every Tandem mode,
including `dev`, `new-instance` and MCP. Its exports override inherited values for that process;
your current shell stays unchanged. The file is trusted shell code. A missing file preserves the
inherited environment. Invoke the built binary directly to use another environment. Cargo test
binaries and release helpers use their inherited environment.

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

The command checks the branch and local Tuicore path, then runs the release workflow's formatting, script tests, Clippy, and Rust tests before changing the version. Cargo validation uses two compilation jobs, two test threads, stripped debug data, disabled incremental compilation, and the persistent `target/release-check` directory. It then bumps Tandem, refreshes and commits the local lockfile, creates an annotated tag without opening an editor or pager, and atomically pushes `main` and its `vX.Y.Z` tag. Generated `Cargo.lock` changes are accepted; all other files must be clean. It returns without waiting for GitHub Actions. Existing `cargo patch`, `cargo minor`, and `cargo major` aliases use the same release flow.

GitHub Actions resolves the highest stable, non-yanked Tuicore version from crates.io before running Cargo checks and builds. This only modifies the disposable CI checkout; local development always builds the sibling `../tuicore` checkout. Registry or compatibility failures stop the build.
