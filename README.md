# Tandem

Keyboard-first local development environments, shared by a TUI, CLI and MCP agents. Templates are editable
development recipes; each instance gets its own workspace and, when services are configured, a Compose project. A label-driven Traefik
gateway exposes declared HTTP services at `http://localhost:9876/<instance>/<service>/`.

## Quick start

Building requires Rust and the sibling `../tuicore` package. Service templates require a local Docker
Engine and Docker Compose v2 or newer; container images are pulled on first use. Repository-only
templates require Linux and host Git but their lifecycle does not require Docker.
Blank and guidance-only templates prepare a workspace folder without Git or Docker.

```bash
cargo run                   # TUI
cargo run -- dev            # development TUI + HTTP MCP at http://127.0.0.1:7348/mcp
cargo run -- mcp            # protocol-only stdio MCP
cargo run -- serve          # HTTP MCP at http://127.0.0.1:7345/mcp
```

`dev` is available in debug builds. It replaces a prior development server using the same Tandem home
and port. Release builds, including the installed binary, expose `serve` for HTTP MCP and launch the
TUI with `tandem`.

1. Press `T` or use the Template button to create a template named `website`; its folder contains only `tandem.json` with `{}`.
2. Press `Enter` on a template, instance or service to view its details. Template details include
   metadata, available Compose source and the manifest.
3. Press `i`, enter `review`, then Enter or Ctrl+S to confirm execution; watch progress in Details.
   **Start instance**, the last dialog control, defaults on. Turn it off to prepare repositories,
   guidance and Compose configuration without starting services; an initial prompt still opens OpenCode.
4. Expand the template row using the DataView's configured expansion key (shown in its action bar)
   and select `review`; the blank workspace is ready and contains standard `AGENTS.md` guidance.
   Add repositories, `compose.yaml`, routes, or `tandem-agents.md` to the template as needed; use a new
   instance name when switching from workspace-only to container-backed execution.
5. For container-backed instances, press `s` on an instance or service to confirm stopping it when running, or starting it when stopped.
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
    Routed services use saved launch-time readiness assertions; imported instances without saved assertions
    require current template readiness configuration. Applying template or image
   changes requires startup.

Within an instance subtree, `s` and `r` target the selected service on service rows and the instance
on other nested rows, including the Services parent. Setup jobs do not support service start, stop,
or restart. Press `p` anywhere in the subtree to confirm purging the instance.

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
Accepted purges immediately hide their rows while cleanup runs. Selection moves to the next surviving
row in the original order, or the previous row at the end. Failed purges restore their rows and report
the error without taking selection back. With OpenCode integration enabled, verified client panes close
before resource deletion; recorded workspace ownership lets this start before Docker inspection.
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
one-second pause to establish CPU usage. Automatic resource sampling runs every five seconds,
independently of terminal focus and the inventory schedule. The header publishes CPU temperature,
available RAM, and combined Docker/OpenCode memory and CPU totals together for each resource batch.
Temperature is shown when an Intel CPU package sensor is available. New or restarted containers
establish their CPU baseline on the next sample. Manual Refresh bypasses the interval, publishes memory
immediately after the first reading, and takes a second reading after one second for current CPU usage.
Sampling requests are serialized, including both readings; slow collection can delay a batch.
Creating/starting instances show `—` until their operation
ends; a failed startup still permits metrics for surviving running containers. Paused containers retain
memory readings with CPU `— · paused`. Collapse, filtering and scrolling leave sampling/totals intact.
Partial totals disclose missing coverage; stale readings retain their age and lose pressure coloring.
Memory pressure is amber at 70% and red at 90% of explicit caps; aggregate coloring requires complete,
fresh, capped coverage. Resource errors do not change runtime health.
An empty template list or unavailable Docker daemon is displayed without preventing template editing.
Starting an existing instance name reapplies the same template directory. It refuses names owned by
another template or unmanaged containers.

Instance startup continues when its TUI or CLI closes or its MCP connection ends. Reopening Tandem
with the same home and namespace shows current startup progress or its retained outcome, including
before containers exist. An interrupted startup requires an explicit retry; reopening only observes
state. Service start and restart keep their documented scope.

## CLI inspection

```bash
tandem list-instances
tandem inspect-instance review
tandem list-templates
tandem inspect-template website
tandem list-instances --json
```

All four commands support `--json`. Instance listings show current status, running/expected service
counts, workspace paths, and retained activities. JSON includes runtime evidence and service details.
If Docker is unavailable, retained workspace/startup inventory is printed when available; the command
reports the incomplete inventory on stderr and exits nonzero. JSON also includes `runtime_error`.
`inspect-instance NAME` shows that instance's description, paths, repositories, service states,
healthcheck observations, URLs, recorded readiness timestamps, and retained startup progress/results.
It can show retained failure evidence even when no runtime instance is observed. A failed operation
is inspection data; incomplete observation or a missing instance makes the inspection command fail.

Template listings include descriptions, directories, execution kinds, and configuration errors.
`inspect-template` shows the template's paths, manifest, Compose source, optional guidance, templates
root, and configured gateway URL. The URL is configuration, not a health check; inspection does not
provision repositories or start services. Missing or invalid templates fail with a nonzero exit code.
Source output may contain secrets from template files; handle it accordingly.

## CLI instance lifecycle

```bash
tandem new-instance review --template website
tandem new-instance review -t website --opencode
tandem new-instance review -t website -o "Explain this project"
tandem new-instance review -t website --start-instance=false
tandem start-instance review
tandem stop-instance review
tandem restart-instance review
tandem delete-instance review
tandem delete-instance review --headless
```

For a new instance, the template must exist. Invoking this command authorizes host Git provisioning
and Docker execution; use trusted templates. Creation follows the saved Branch instances setting and
the same ownership checks, locks and startup rules as the TUI/MCP. The CLI waits up to ten minutes for
startup readiness, prints the workspace and service URLs on success, and exits nonzero on failure.
Failed startups preserve resources for inspection.

`--start-instance` defaults to true. Set it to false to complete workspace and Compose preparation
without Compose startup, gateway startup or service-readiness waits. Service instances remain
container-backed and report **Not started**; use `start-instance` to start them later.

`start-instance` requires an existing managed instance and its trusted template. It uses the TUI's
startup path, provisions missing repositories, reapplies Compose configuration, and may rerun setup
jobs. Existing checkout edits, branches, descriptions, and workspace guidance are preserved. Start
waits up to ten minutes for readiness and continues in the background if its CLI closes. Unpause
containers before starting; use a new instance name to change execution kind.

`stop-instance` waits up to one minute for containers to stop and preserves workspaces, volumes,
networks, templates, and the gateway. Workspace-only instances are preserved without container work.
`restart-instance` restarts existing long-running containers, including stopped ones, while preserving
data and configuration and skipping setup jobs and the gateway. It waits up to ten minutes for
configured healthchecks and route content assertions; services without checks are verified as running.
Restart applies no template edits or builds. Routed services use saved launch-time readiness assertions;
imported instances without saved assertions require current template readiness configuration.
Workspace-only instances have no restart targets; paused containers require unpausing.
Invoking these lifecycle commands authorizes their scoped operations; failures exit nonzero.

`new-instance` leaves an existing instance owned by the requested template unchanged, including when
stopped or paused. It reports that the instance exists without checking readiness. With `--opencode` or `-o`,
it opens a fresh OpenCode client in that instance's workspace; template files and repository sources
are not needed. Ownership mismatches and concurrent instance operations are rejected.

`--opencode` (also `-o`) opens a blank client when used alone and accepts optional initial prompt text.
Quote multiline or multiword prompts; use `--opencode="--text"` for text beginning with a hyphen.
Without the flag, creation opens no client. `--description` (also `-d`) sets the new instance description.
OpenCode launches require Tandem to run inside Zellij with the OpenCode integration enabled.
CLI client launches use an observed instance pane's tab for a stacked pane, otherwise Tandem creates
an instance-named tab. The TUI's **New OpenCode session** opens a session tab in an existing V2 client;
without a tab-capable client, it uses this pane placement and launch logic.
A known workspace server is reused; otherwise OpenCode starts in the workspace.
Client launches run through `direnv exec .` when `direnv` is on the pane's `PATH`, loading the
workspace's approved environment, including inherited station configuration. Without direnv,
OpenCode launches directly. A direnv failure stops the client launch and appears in the pane;
Tandem never automatically approves `.envrc` files. Attaching to a shared server keeps that server's
existing configuration.

For a new instance, Tandem creates and validates the workspace directory, completes creation-history
cleanup when enabled, and launches the requested OpenCode client before cloning repositories.
Seed copying and workspace `AGENTS.md` generation precede the launch. Repository preparation,
verified guidance mappings and requested container startup follow it. Initial prompts run without
waiting for instance readiness. Startup continues if the client launch fails.
Client lifetime does not delay CLI exit. Launch failures make the CLI fail after startup completes,
or immediately for an existing instance; prepared workspaces remain available.

Session instructions and initial prompts require the bundled Tandem TUI companion. The release shell installer refreshes it and
prints its OpenCode `cli.json` plugin directory (V1: `bridge.mjs` in `tui.json`); after source or manual updates, run `tandem opencode-setup`.
Ensure the printed entry is in the plugin array. Each new client receives
`TANDEM_INITIAL_PROMPT` and instance-state instructions; the companion consumes and clears them,
waits for client readiness, creates a conversation and attaches instructions before any initial prompt.
Without prompt text, instructed instance sessions open without requesting a model reply.
Ordinary sessions use the configured agent/model defaults; rule sessions use their pinned model/variant.
The prompt travels as a literal environment value; avoid including secrets.
CLI success confirms requested preparation/readiness and pane launch, not prompt delivery or model completion.
The companion reports submission failures in the new client and does not retry uncertain requests.

Every new instance session opened through `n` or creation receives current service-state guidance.
Starting services allow code exploration during preparation and require readiness only when the work
needs them. Started services require readiness verification when needed. Stopped or prepare-only
services require asking the user before starting them. Opening a session does not start containers.
V2 stores this guidance as a durable `tandem.services` instruction entry; V1 uses synthetic context
with no model reply. Generated workspace guidance lists configured services and preserves user edits.

`delete-instance` permanently removes the named instance's owned containers, workspace, private volumes,
networks, private rendered Compose file, and local instance records. It leaves templates, shared images, and the gateway intact.
`--headless` (also `-h`) launches deletion in a detached Tandem process and returns after that process starts.
Inspect diagnostic logs or runtime state for the detached deletion result.

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

Every new instance receives a root `AGENTS.md` after seed copying and before any requested OpenCode launch
or repository cloning. Early guidance lists declared repositories and previewed services; preparation
refreshes verified mappings before container startup if the newly generated file remains unchanged.
OpenCode launches in instance workspaces also generate it if missing; generation failures block the launch.
Existing regular files and client edits during preparation are preserved, while symlinks and directories are
rejected. This is a configuration snapshot: keep local guidance current as repositories or services
change. To regenerate from the editable template, move the workspace file aside and launch OpenCode in the instance.
Template-owned clone jobs and container-created files appear only after their own setup completes.

## Events and developer providers

**Events** is the second main tab. It shows the latest 200 received events in arrival order, with
new arrivals at the bottom, using distinct message, ticket, system-event, and generic rows.
Search by provider, stream, profile, summary, or event ID.
Each event occupies two lines: a type glyph, provider identity, muted `#<sequence>` local event ID,
and profile-specific fields above the
message text, ticket title, system description, or generic summary. The glyphs are `` message,
`` ticket, `` system event, and `` generic; Providers uses the same glyphs. The glyph is green
when the latest processing attempt is accepted and uses the normal text color otherwise.
Message headers mark nonblank thread references with `󱡠`; ticket headers omit missing or blank
assignees. System headers show resource, optional environment, signal, and severity in that order.
Environment aliases `prod`, `dev`, and `stage` display as `production`, `development`, and `staging`;
custom labels remain intact. Separators and secondary fields are muted; severity colors only its label.
Press Enter to inspect the normalized payload, metadata, supporting context, and the latest 50
processing attempts. The feed lists newest events first and starts with top-following enabled. Selecting
the first row enables following; moving to an older row pauses it and retains that event during updates.
The `gg` toggle controls following. `gg` in the DataView selects the newest matching event and
resumes following. `Shift+H` opens the Sessions overview from any main tab, including during search.
It clears searches and stream filters on every page and selects each DataView's first item.
Events enables handover filtering and resumes following.
With OpenCode integration disabled, it opens the Instances overview.
Press `.` for the highlighted event's action menu with hotkeys; `p` selects its source in Providers.
`x` deletes the highlighted event, including from the `.` menu. `X` deletes all retained events
in the current namespace, across providers and filters. Both require **Ok** (`o`) or **Cancel** (`c`).
Deletion removes event history, feedback, and pending rule work while preserving instances, sessions,
rules, provider credentials, and dispatch rate limits. Active rule actions must finish first.
Redelivering a deleted event creates a fresh receipt and can trigger enabled rules again.
The stream multiselect at the top (`P`) lists `provider · stream` and filters by exact pairs. Enter toggles an option,
Ctrl+J/Ctrl+K moves through options, and Ctrl+Enter applies the selection. An empty selection shows
all streams. A stream row's **Stream events** action selects only that stream and clears event search.
`Shift+Enter` clears the stream selection. The `T` toggle
shows only events handed over to Tandem instances and is enabled at startup; acceptance alone does not establish assignment. The count
at the right shows displayed events out of the full retained history. Changing a stream or handover filter selects the newest matching
event and focuses the DataView. Escape or Ctrl+[ from the filters or toggles returns to the DataView.

The first Events toggle, **󰕾 acceptance sound** (`Shift+N`), is off at startup. It plays for newly
accepted processing attempts across all providers, independently of filters and the active tab.
Startup history is silent; replay can produce a new notification. Acceptances in the same refresh
share one sound. `Shift+H` turns this toggle off.

Events start **pending**. **Accepted** means a rule matched the event and triggered its action;
instance startup, prompt delivery, and task completion have separate outcomes. Acceptance belongs
to automatic rule processing. `r` replays any retained event as a new pending attempt while preserving
its original event ID and history. Replay requires approval to evaluate current enabled rule revisions;
use it to apply newly created or enabled rules to earlier events. Each replay can trigger the same rule
again with a fresh instance. The row shows `Accepted · 3 rules` for three matches in
the latest attempt. The **Acceptances** details tab lists each rule, timestamp, attempt, assigned
instance, session, and dispatch outcome. Schema upgrades require existing Tandem clients and
sidecars to be stopped; retained processing history keeps its identities and timestamps.

**Providers** discovers packages under `provider_templates_root` and displays a provider → streams tree.
Provider rows show identity, handed-over/total event count, collector status, and collecting-stream count. Counts cover
distinct events across full retained history for the manifest's
provider identity. Select a provider and press `.` for its action menu: **Start provider**/**Stop provider** (`s`),
and **Logs** (`o`). The hotkeys choose
the action for the observed state and also work directly on the row; unavailable menu actions are muted.
Enter inspects its configuration and runtime state. Lifecycle actions ask for approval because they
execute trusted Docker code.

Stream children show their own handed-over/total counts and collection state. Enter opens **Stream details**;
`.` offers **Start stream**/**Stop stream** (`s`) and **Stream events** (`e`). Provider actions target the
collector and all its declared streams. On a running compatible provider, stream actions preserve siblings.
Starting an individual stream on a stopped or uninstalled provider starts its collector with only that stream
enabled. Stopping the last enabled stream also stops the collector, preserving checkpoints and history.
Stop cancels collection inside the collector and clears that stream's buffered events. Resume is
live-only: the stopped interval is skipped, without catch-up. Completion requires the collector's current
acknowledgment within ten seconds. Enabled streams show **Starting...** while waiting for that acknowledgment;
timeout, errors or expired evidence show **Unverified**. The periodic control acknowledgment verifies readiness
without waiting for a source event or adding health-check events to history. Streams inferred from
retained events remain inspectable when their provider does not declare stream control. Logs belong to the
collector and open in a bottom dialog. Tree expansion uses the DataView's configured bindings and survives snapshot refreshes.

The right-side control is **Start all**/**Stop all** (`S`). It shows Stop when any provider is
running or paused, and Start when none are running or paused. Bulk actions
confirm the eligible, idle providers before running; unavailable providers are skipped. Search input
retains uppercase letters without triggering these actions.

Start provisions private credentials, starts
the Tandem-owned sidecar, builds the image, and verifies the owned collector is running.
There is one installation per package in each namespace. Manifest identities are unique and fixed
for installed packages. CLI/MCP Start on a running provider preserves its configuration; apply template
edits by stopping and starting it. Running proves container liveness, not successful source collection.
Provider Start enables every declared stream, including streams individually stopped while the collector ran.

Stop preserves checkpoint volumes and event history. Stop discards incoming events as soon
as its operation starts, before Docker work; discarded events create no history or feedback.
Failed Stop operations keep ingestion disabled; unavailable actions preserve the prior gate.
Start enables ingestion before collector work and restores the prior gate on failure.
Start unpauses an owned paused collector and verifies it is running and unpaused.
Lifecycle actions verify namespace, provider, Compose project, and
collector-service ownership before mutation. Retained launch snapshots permit Stop and Logs even
when a package directory is removed. Build/start failures and Docker observation failures remain
visible; unavailable Docker is not evidence that a provider is stopped.

MCP `delete_provider` accepts the package `name` from `list_providers` and requires `confirmed: true`.
It permanently removes the shared package, owned collector and checkpoint volume, credentials, stream
controls, events, attempts, acceptances, and feedback. It also purges event-created instances, their
workspaces and associated OpenCode sessions, independently of history-cleanup preferences.
Shared rule definitions, sidecars, namespace dispatch-rate history and Docker images/build caches remain.
Active rule actions, another namespace's installation, unsafe paths or unverifiable/reused instance
identities block deletion. External cleanup can partly succeed before a failure; identity and history
remain available for retry, with ingestion, collection restarts and queued dispatch blocked.
Source-system side effects are outside this local purge. Close old Tandem clients and sidecars before
upgrading so all processes honor deletion coordination.

Tandem owns one detached sidecar per home/namespace, separate from MCP. It chooses a loopback port
and passes that address to collectors. A private process lease and identity probe prevent duplicate
workers and verify readiness. Providers and collection continue after the TUI closes. Opening Tandem
supervises the sidecar for active providers. On Linux, supervision, provider Start and rule activation
ensure the receiver uses the installed Tandem executable. A verified outdated receiver shuts down
gracefully before replacement on its recorded address, so collectors reconnect with their checkpoints,
credentials and event identities intact. Stop controls collectors; the shared sidecar stays running.
Private worker diagnostics live at `$TANDEM_HOME/provider-sidecar.log`. Provider process logs are
available through the Providers tab. Normal operation needs no Compose command, token setup, or
manual sidecar startup.

Provider ingestion and feedback require a provider-scoped `Authorization: Bearer <token>` header;
browser-origin requests are rejected. The owned sidecar has no MCP route. Its read-only
`/v1/identity` probe contains process identity, not credentials. The HTTP transport has no TLS and
managed collectors use Linux Docker host networking. `serve-events` is an optional foreground
protocol-testing server, independent of the owned lifecycle.

| Endpoint | Contract |
| --- | --- |
| `POST /v1/events` | JSON object with an `events` array of 1–100 events; atomic acceptance returns `receipts` with event ID, local sequence, and duplicate flag. Disabled ingestion returns HTTP 200 with empty `receipts` and a `discarded` array containing the batch's event IDs. |
| `GET /v1/notifications` | Returns up to 100 unacknowledged notifications for the authenticated provider, oldest first. |
| `POST /v1/notifications/{id}/ack` | Idempotently acknowledges that provider's notification; returns 204. |
| `GET /v1/streams` | Returns a `streams` array of the authenticated provider's `{stream, enabled, revision}` controls. |
| `POST /v1/streams/ack` | Acknowledges one exact current control after the collector applies it; returns 204, or 409 for unavailable/superseded controls. Repeat applied acknowledgments at least once per second; freshness expires after five seconds. |

Provider manifests declare `streams: ["messages", "reactions"]` and `stream_control: true` when the
collector supports the control contract. Poll controls at least once per second, even when every stream is
stopped. Cancel and join a stopped stream's collection work before acknowledgment; filtering emitted events
alone is insufficient. Sibling streams continue. Start uses the current source position and skips buffered
history. Manifest stream names are unique, nonempty strings up to 80 bytes without control characters.
Ingestion racing with stream Stop is discarded for that stream; mixed batches can contain both receipts and
discarded IDs, each in input order. This ingestion guard does not certify collection has stopped.

Each event has `schema_version: 1`, a stable `event_id`, `stream`, namespaced `type`, `summary`,
`profile`, and normalized `data`. Optional fields are `occurred_at` (RFC 3339), `subject`, `body`,
`url`, `people`, `attachments`, `relations`, `context`, and object-valued `metadata`.

| Profile | Required normalized data | Optional normalized data |
| --- | --- | --- |
| `message` | `author`, `channel`, `text` | `thread` |
| `ticket` | `key`, `title`, `status` | `assignee` |
| `system_event` | `resource`, `signal`, `severity`, `description` | `environment`, `severity_color` |
| `generic` | Any JSON object | — |

System severity is nonblank free text up to 80 bytes; optional environment is free text up to 200 bytes.
Both reject control characters. Source labels are preserved in storage; blank environments are omitted
from rows. Optional `severity_color` selects `plain`, `info`, `warning`, `error`, or `success` and takes
precedence over automatic coloring. Without an override, exact trimmed, case-insensitive labels select
the color: info/information/informational/notice/debug/trace use info; warn/warning/caution use warning;
err/error/critical/crit/fatal/severe/emergency/emerg/alert use error;
success/successful/ok/okay/done/complete/completed/passed/resolved/healthy use success.
Unknown labels use normal text color. Severity coloring is independent of the processing attempt.

People contain `id`, `name`, and `role`; attachments contain `name`, `url`, and optional `media_type`;
relations contain `kind`, `subject`, and optional `url`. Attachment references are displayed without
fetching their contents.
The developer packages provide complete examples, including thread context and custom metadata.

Events are bounded to 64 KiB each and requests to 1 MiB. Deduplication is scoped by namespace,
provider installation, and provider event ID. During acceptance, a matching ID with different content
rejects the whole batch with 409. Providers must persist a batch before sending it and advance
checkpoints only after receiving its acceptance receipts or matching discarded IDs.
A discard acknowledgment is terminal, not a storage receipt; providers must not retry those events
after resuming. While ingestion is enabled,
historical observations and distinct events in a burst are all accepted; source timestamps do not
suppress them. Stored history has no automatic pruning in this developer slice.

Tandem durably queues `received`, `replayed`, and `acknowledged` feedback with stable notification
IDs, original event IDs, local sequences, and processing-attempt IDs. Providers poll and acknowledge
these notifications. Delivery retries and sidecar restarts preserve the queue; providers must
deduplicate notifications and reconcile their external side effects before acknowledgment.

## Event rules

**Rules** is a main tab after Instances. Each rule occupies two rows showing its enabled state,
name, handler template, model, and description. `a` activates or deactivates the selected rule; Space
also toggles it. `.` opens its action menu with hotkey labels. The top-right `A` button offers
**Activate all** when no rules are active, or **Deactivate all** when any rule is active.
Single-rule activation and deactivation save immediately and report success or failure in a notification.
Activation authorizes automatic execution. Bulk actions confirm eligible saved revisions with **Ok (o)**
and **Cancel (c)**. A white pause icon marks inactive rules; a green play icon marks active rules.
TUI activation binds the rule to that Tandem process's current Zellij session. Activation and dispatch
require a live target; dispatch checks before preparation and again before launching. If the session
closes, deactivate and reactivate the rule in the desired Tandem TUI. Existing acceptances keep their
pinned destination; an explicit replay after reactivation uses the newly authorized revision.
Enter opens a bottom-docked dialog with **Accepted events**,
**Script**, and **Settings** tabs. Valid field edits save automatically while the dialog stays open;
invalid input stays editable with a save error. Script uses Rust highlighting for Rhai syntax;
Initial prompt uses Glimmer highlighting for Handlebars syntax.
Editing an enabled rule pauses it before saving the draft; `a` in Rules authorizes its updated
revision. Already accepted work retains its pinned definition. Predicates inspect provider-supplied
custom fields through `event.metadata`.
In a rule's acceptance list, Enter switches to Events and selects that exact retained event, clearing
conflicting filters even outside the latest-200 feed. In an event's acceptance list, Enter selects its rule.
Event dialogs open on **Acceptances**, followed by **Event details**.
The `.` menu offers **Go to instance** (`i`) and **Go to OpenCode session** (`o`); both keys also work
directly on the row. Session navigation is available once a conversation or assigned pane is recorded.

Create rules through MCP with `save_rule`. Use `list_rules`/`get_rule` to read revisions and acceptance
history, `list_events`/`get_event` to inspect input, and `preview_rule` to check matches and resolved
prompts without contacting an agent or creating an instance. Updates require the saved revision.
Shared definitions live in `$TANDEM_HOME/templates/rules/<name>/rule.json`, alongside the instance
and provider catalogs in the template Git repository. Each file contains `name`, `script`, `template`,
`model`, and `initial_prompt`, with optional `description` and `start_instance` (default true).
The name must match its directory. Activation (`enabled`), Zellij targets, revisions, and execution
history live locally in `settings.sqlite3`; omit `enabled` from the file and set it through `save_rule`
or the TUI. File additions are discovered automatically as inactive rules. Direct edits pause the rule
in each namespace and require activation of the updated revision. Removing a file stops future
matches; pinned work and history are preserved. Invalid files surface errors and block new attempts
until corrected.

```rhai
fn matches(event) {
    event.profile == "message" && event.data.channel == "support"
}
```

The predicate receives the normalized envelope plus authenticated `provider`, local `sequence`, and
`received_at`. All four profiles expose `data`, shared context, and arbitrary event metadata. Rhai
returns only a boolean; Tandem owns side effects. Evaluation limits include 50,000 operations,
32 call levels, bounded expressions and collections, and no exposed host filesystem, network, or
process capabilities. Script errors remain visible and do not stop sibling predicates.
Use `sample(event.event_id, 0.8)` for an approximately 80% deterministic sample. The same nonempty
key always produces the same draw across previews, retries, and restarts. Keys are limited to 1024
bytes; the probability must be a finite decimal between `0.0` and `1.0`. Sampling is not a security
or rate-control mechanism. Prefix the key with a rule name to give different rules independent draws.

Prompts use Handlebars with nested paths, `if`/`unless`, `each`, `with`, comparison helpers, and
inline partials. `{{event.data.text}}` inserts literal text; `{{event}}` and event-path collections
render as JSON. `{{json event}}` explicitly serializes any value, including within loops.

```handlebars
Message: {{event.data.text}}
{{#if event.data.thread}}Thread: {{event.data.thread}}{{/if}}
{{#each event.attachments}}
- {{name}} ({{url}})
{{else}}
No attachments.
{{/each}}
```

Event text stays literal and HTML escaping is disabled. Missing interpolated fields are errors;
`if` can guard optional fields. Template syntax is checked on save. Rendering permits 32 nesting
levels and 50,000 template evaluations, including empty loop bodies. Templates are in-memory and
expose no filesystem, network, or script helpers. Empty/NUL-containing results or output above
64 KiB fail that action while retaining its acceptance.
Models use `provider/model#variant`, with the variant optional. Model syntax is validated locally;
availability and credentials belong to the configured OpenCode environment. V2 rule launches require
`opencode-station run` with model/prompt/variant support; V1 launches use `opencode`.

Authorization covers every future match of that enabled revision, including trusted template code,
host credentials, model costs, and creation-history cleanup when enabled. External text is task data,
not authorization. Rules are namespace-local SQLite records. New attempts pin enabled revisions;
edits and toggles affect future attempts while queued actions retain their saved definition and prompt.
Every matching rule creates its own fresh instance. Dispatch waits for successful instance readiness,
then requests a new conversation with the configured model and prompt. Disabling a rule does not
cancel already accepted work.

A namespace lease serializes automation across processes. Dispatch admits four active actions and
ten starts per minute, including retries. The owned event sidecar processes work after TUI/MCP clients
close. Acceptances retain queued, provisioning, launching, launched, failed, or uncertain outcomes.
`launched` proves observed session/pane linkage, not prompt completion or task success.
Provider `assigned` feedback includes dispatch ID, rule name/revision, and instance; it proves durable
startup admission, not readiness or prompt delivery.

Use confirmed `retry_acceptance` only for pre-launch failures after correcting their cause. It keeps
the assigned instance and successful siblings. Uncertain launches are never automatically resent;
inspect their workspace/pane before deliberately replaying. Confirmed `replay_event` uses current
enabled rules under a fresh attempt. Reuse its `request_id` when resubmitting the same request.
Providers must exclude their own feedback-driven source updates to prevent automation loops.

See [developer provider setup](projects-generators/README.md#event-providers) for the four Docker
images and isolated verification commands.

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

Workspace-only instances (blank, repository-only, or guidance-only) use a folder icon after
preparation and guidance generation, retain their workspace path, and show a single subtle
**(no services configured)** child when expanded. They hide
CPU/memory metrics and disable container Start/Stop/Restart actions; OpenCode, Details and Delete remain
available. Failed preparation retains completed checkouts and reports its error; retry New instance
with the same template/name after correcting the cause. Docker failures do not make these workspaces stale.

## Agent workflow

```json
{"command":"tandem","args":["mcp"]}
```

The MCP tools are `get_instructions`, `list_templates`, `get_template`, `create_template`, `update_template_manifest`,
`list_instances`, `create_instance`, `get_operation`, `stop_instance`, `delete_instance`, `start_service`,
`stop_service`, `restart_instance`, `restart_service`, `list_providers`, `provider_action`, and `get_status`.
Provider actions are `start`, `stop`, and `logs`; lifecycle actions require
`confirmed=true`. CLI equivalents are `tandem list-providers` and `tandem provider <action> <name>`;
they use the same service boundary and perform the sidecar/credential setup themselves.
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

A template contains `compose.yaml`, `tandem.json`, `tandem-agents.md`, `tandem-files/`, or a combination. Compose-only
templates use empty manifest defaults: services have no Tandem routes, one-shot roles, or managed clones.
Repository-only templates omit `compose.yaml`, declare at least one repository, and omit routes and
one-shots. Guidance-only templates supply `tandem-agents.md` with an optional manifest and prepare an
otherwise empty workspace containing generated `AGENTS.md`. Blank templates contain only `tandem.json`
with `{}` and prepare the same workspace shape. Empty unrelated directories are not templates, and an
empty Compose services map is invalid. The template creator supplies a blank template;
edit its dedicated directory to add capabilities. Existing instances retain their launch
kind; use a new instance name to switch between workspace-only and container-backed execution.

```text
<TANDEM_HOME>/templates/instances/website/
  tandem.json
  compose.yaml                   # optional services
  tandem-agents.md                # optional template guidance
  tandem-files/                   # optional files copied into workspace root
  scripts/                       # optional, edited by the agent
  config/                        # optional, edited by the agent
```

Relative mounts, builds, env files, and scripts resolve against the template directory.
Generated configuration and instance records live in Tandem's private local storage.
Tandem uses the nested instance catalog when `templates/instances/` exists. Flat instance catalogs
remain supported and are preserved; mixing flat and nested instance recipes fails initialization.
Fresh developer fixtures put instance and provider packages in `templates/instances/` and
`templates/providers/`; rule definitions live in `templates/rules/`, sharing the Git repository at
`templates/`. `get_instructions` returns the effective instance, provider, and rule roots and the
shared repository root.

`tandem-files/` seeds each instance workspace with its contents, including nested directories, hidden
files, binary files, and file permissions. Copying runs before workspace guidance generation, any
creation-requested OpenCode pane launch, and repository cloning. Existing regular files are preserved during preparation retries;
incompatible destinations fail preparation. Ready workspaces remain user-owned when the template changes.
Paths must be UTF-8; symlinks and special files are refused. Seed paths must not occupy repository targets,
root `AGENTS.md`, root `.tandem-*`, or any `.git` path. Files-only templates prepare a workspace and
generated guidance without a manifest, Git, or Docker.

Template details show a **Files** tree tab when the folder contains files. The background observer
refreshes the tree when entries change or the folder is removed; an empty folder hides the tab.
`tandem inspect-template <name>` lists its directory and tree; JSON and MCP template inspection expose
`files_directory` and `files` with relative paths, entry types, sizes, and modification timestamps.

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

### Shared recipes and local state

Keep `$TANDEM_HOME/templates` under Git for team-shareable recipes, scripts, static assets, and non-secret
seed files. Instance ownership, preparation and lifecycle records, startup outcomes, and launch snapshots
live in local `$TANDEM_HOME/settings.sqlite3`. Generated Compose files live in private
`$TANDEM_HOME/runtime/<namespace>/<instance>/compose.json`; Docker supplies observed container state.

Compose resolves relative paths from the template directory. Mount static template assets read-only.
Tandem rejects writable binds and local bind-backed volumes exposing the templates root, its descendants,
or ancestors, including symlink aliases. Missing template mount sources and local build-cache exports into
templates are rejected. Trusted privileged containers and externally configured Docker resources still
require care; this is not a container sandbox.

Put package caches, temporary files, logs, and application data in the instance workspace or project-owned
named volumes. Workspace `.local/{cache,tmp,logs,data}` is a convention, not automatic configuration.
Set host cache paths to host absolute paths and container cache paths to mounted container paths.
Shared caches require an explicit location, permissions, and retention policy. Keep secrets in private
external files, even when an in-repository path would be ignored. Before creating an instance, use a private
host temporary directory outside templates and repositories for experiments.

Close old Tandem clients and workers before the first launch after upgrading storage. Initialization
validates and imports legacy files under operation locks, retains private backups under `runtime`, and
refuses conflicts or unverified ownership. Imported legacy files are removed when writable; read-only
checkouts do not block import, and their cleanup is retried on later launches. SQLite remains authoritative
after import. Existing application caches and data are user-owned and are not moved or removed automatically.

MCP `get_instructions` returns bundled `core_guidance` alongside editable `markdown`. The placement baseline
reaches installations with custom instructions without overwriting them; agents must ask before acting on
conflicting guidance. Existing workspace `AGENTS.md` files remain user-owned.

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
| `TANDEM_KEY_START` | `i` | New instance dialog |
| `TANDEM_KEY_NEW_TEMPLATE` | `T` | Create template dialog |
| `TANDEM_KEY_STOP` | `s` | Start/stop the selected instance or service; stop all instances on a template row |
| `TANDEM_KEY_REFRESH` | `R` | Refresh template/runtime inventory |
| `TANDEM_KEY_DELETE` | `x` | Delete the selected template with all its instances and data |
| `TANDEM_KEY_PURGE` | `p` | Purge the selected instance, or all instances of the selected template |
| `TANDEM_KEY_RESTART` | `r` | Restart the selected instance or service after confirmation |
| `TANDEM_KEY_STOP_ALL` | `S` | Stop instances across all templates after confirmation |
| `TANDEM_KEY_PURGE_ALL` | `P` | Purge instances across all templates after confirmation |

Application hotkey overrides accept distinct ASCII letters; `TANDEM_KEY_INFO` also accepts `Enter`.
Shared navigation, focus, and component keys use tuicore configuration. The TUI displays resolved key labels.
Dialog actions use **Ok** (`o`) and **Cancel** (`c`); the top-right close control uses `x`.
While editing text or searching, letters belong to the active input.

### Workspace actions

The **New instance** dialog accepts a name, a multiline description, and an optional initial prompt.
Description and prompt content grow from 2 to 8 rows, then scroll. A nonblank prompt opens OpenCode
and submits the text when the workspace is prepared; empty or whitespace-only prompts leave OpenCode
closed. Launching requires the OpenCode integration and a Zellij session. `Ctrl+Enter` submits the
dialog, except while editing a textarea: the first press returns it to focus mode, and a second
press submits. An existing name selects that instance and leaves its sessions unchanged.

Press `y`, or choose **Yank** from the `.` Actions menu, to open the copy menu for the selected
instance or routed service. For an instance, `i` copies its name and `w` copies its absolute workspace path;
for a routed service, `u` copies its URL.
Choose **Open in browser** from a routed service's
`.` Actions menu, or press `Ctrl+Enter` on that service, to open its URL. On an instance,
`Ctrl+Enter` opens its only routed service directly or displays a chooser when several are available;
each chooser option is shown as `<service> - <url>`.

### Completion settings

**Completion fade** accepts whole seconds from 1 to 3600 and defaults to 20. It controls the
gutter marker's final fade; the two short pulses and 150 ms rise keep their fixed timing.
Valid edits save immediately in `$TANDEM_HOME/settings.sqlite3` and apply to active markers.

**Completion sound** and **Event acceptance sound** independently select installed `.oga`, `.ogg`,
and `.wav` files from the user and system
XDG sound directories. Confirming a dropdown selection saves it and plays a preview, even when
its toolbar sound toggle is off. Moving the highlight is silent. Playback is
asynchronous; a new preview replaces the previous sound. System default uses the desktop's
`complete` event; an unavailable saved choice falls back to it. Restart Tandem to discover
newly installed sounds. Playback failures are recorded in diagnostic logs.

### OpenCode integration

**Clear OpenCode history on new instance** in Settings defaults to on and persists across restarts.
With OpenCode integration enabled, creation permanently deletes saved conversations for the exact
workspace directory before provisioning or launching a client. Reusing a deleted instance name starts
with clean history; reopening an existing instance, attaching a session, and retrying provisioning
preserve conversations. Subdirectories and similarly named workspaces are separate scopes.
Cleanup refuses known attached clients, busy conversations, pending questions, and child conversations
outside the workspace. Close active clients before recreating an instance. API failures block creation;
correct the error and retry, or turn off the setting to retain history. Completed deletions cannot be
undone if later provisioning fails.

Cleanup uses discovered directory servers or starts a temporary local `opencode serve` process with
plugins disabled when none is discovered. OpenCode must be available on `PATH` for that fallback;
the process is stopped after cleanup. Cleanup is bounded by 60 seconds and the creation deadline,
and refuses inventories of 10,000 or more sessions rather than risking incomplete deletion.

**Enable opencode integration** in Settings defaults to on and persists across restarts.
Turning it off hides the integration rows, counts, and actions and cancels Tandem's observation
and navigation tasks. Existing OpenCode servers, conversations, panes, and workspace commands
remain under their current owners. Re-enable it to request a fresh observation.

The release shell installer installs the companion after placing the Tandem binary and prints the
OpenCode plugin directory. For source builds or manual refreshes, install it with:

```bash
tandem opencode-setup
```

The command writes the bundled TUI companion under `$TANDEM_HOME/opencode-plugin` and prints
its absolute path. Add that directory to the `plugins` array in OpenCode V2 **`cli.json`**;
V1 uses its `bridge.mjs` file in the `plugin` array of **`tui.json`**. Preserve other entries,
then reopen OpenCode clients. With managed dotfiles, edit the managed source and
apply it. The command leaves your OpenCode configuration untouched. The companion requires the
OpenCode TUI plugin API (verified with OpenCode 1.18.29 and 2.0.22); navigation requires Zellij 0.45 or later.

The V2 companion reports each client's current route and open session tabs reactively. Tandem
watches these receipts and subscribes to local server events for activity and history updates.
Reconnects request a fresh snapshot because the event stream has no replay. A five-second companion
heartbeat maintains navigation freshness; unchanged heartbeats do not request server snapshots.
Tandem joins the reports to live Zellij panes:

| Child-row state | Meaning | Navigation action |
| --- | --- | --- |
| attached · busy | A client has an open conversation that is working or retrying | Goto panel |
| attached · idle | A client has an open idle conversation | Goto panel |
| detached · busy | A conversation is working without an observed pane | Open panel |
| saved | An idle conversation has no observed pane | Open panel |

Multiple panes displaying one conversation appear beneath its child row in the instance tree.
**Enter** on a conversation or pane child opens a
bottom-docked dialog with **Conversation**, **Details**, and **Actions** tabs. Conversation loads
the complete user/agent message history on demand, with newest messages first so the latest
answer and question are immediately available. Text, reasoning, attachment names, and tool-status
summaries are displayed in a wrapped, scrollable, searchable Markdown viewer. Reopen the dialog
to refresh its read-only snapshot. Responses are bounded to 8 MiB; oversized histories produce an
explicit error rather than a silently truncated transcript. Closing the dialog cancels a pending read.

Observed OpenCode permission requests appear as **Approval: Pending** in conversation details.
Pending questions use the **awaiting answer** activity state.

Sessions and clients whose directories do not belong to a Tandem instance appear under **Other
OpenCode workspaces** at the top of the instance tree, grouped by their exact directory. Tandem can
display their details and conversations, create a session, open or jump to a conversation, and close
observed panes. Open conversations follow their native tab positions, independently of activity.
When one conversation is open in multiple clients, its first Zellij-session/pane identity determines
its position. Every open tab is shown; detached conversations use a 20-item recent-history window
per directory with a muted final row when more are available. Saved history follows open conversations.

CPU and memory columns include observed client processes. Tandem reads each distinct client PID's
Linux `/proc` counters on a five-second sampling timer. Memory is RSS;
CPU averages the interval between readings, with 100% representing one fully occupied core.
Memory appears on the first reading; CPU needs two readings for the same process lifetime.
Shared servers and child processes are outside this scope, so detached conversations without a
client have no attributable reading. Summed RSS can include shared pages in more than one process.

Conversation rows sum their clients. Sessions, external-directory, and external-workspace groups
sum their observed processes. Instance and template totals combine clients with container usage;
the toolbar includes both owned and external clients. Services and Setup groups sum their own
containers. Each PID contributes once to a total, regardless of pane nesting, collapse, view filters,
or the session display limit. Agents view retains full instance totals, including services and clients
hidden by that view. A `*` marks partial or stale coverage; details include sample age and client
collection errors. Exited clients leave the totals on the next successful observation.

The following keys work in the normal tree and Agents view while search and dialogs are closed;
the **`.`** menu exposes the same row actions:

| Key | Action | Selected row |
| --- | --- | --- |
| `n` | New OpenCode session | Instance, Sessions group, external-directory group, conversation, or client/pane |
| `o` | Open or jump to the selected conversation/pane | Owned or external conversation, or client/pane |
| `c` | Close the selected Zellij pane | Attached conversation or client/pane |
| `c` | Confirm closing all observed OpenCode panes in the group | Instance, Sessions group, external-directory group, or Other OpenCode workspaces |
| `i` | New Tandem instance | Template, instance, or service |

`n` uses the instance workspace from an instance or Sessions group, and the exact directory from an
external group or OpenCode child. The top-level **Other OpenCode workspaces** group has no creation
action because it spans directories. With a running V2 client and the Tandem companion, `n` focuses
an open empty session in the requested directory, or creates one when none is available, then focuses
its exact Zellij pane. A reusable session is idle and has no messages, queued input, or pending forms;
the companion verifies its cached data before selecting it. Eligible tabs are checked in native order.
OpenCode session tabs must be enabled. A failed tab request reports an error; check the client before
retrying because creation may be uncertain. Without a tab-capable client, creation launches a blank
client without sending a prompt: it attaches to a healthy known local server, or starts OpenCode in
the directory when no server is known. A live destination supplies a stacked pane; a closed or absent
destination creates a named tab in the current Zellij session. Creation immediately focuses the client.
The DataView selects that client's row when observation arrives and reveals its parent groups;
conversation titles in the DataView remain independent of the pane title.

Use **Enter → Actions** to jump to a specific pane or resume a conversation. **Goto panel** and
**Open panel** use `o`; navigation works across tabs and Zellij sessions. Closing a pane preserves
saved conversation history and closes every OpenCode tab in that pane. With the V2 companion,
all open session tabs appear as attached conversations, including background tabs when history is
hidden. `o` selects the matching OpenCode tab before focusing its Zellij pane.
Group closure includes observed panes hidden by filters or display
limits; external-directory groups match their exact directory, and instance groups use workspace
ownership. The external aggregate closes only outside-Tandem panes.
Close and Close all hide the targeted rows immediately while pane closure runs; failures restore them
without stealing selection. Closing all panes in a group runs concurrently. Zellij removes tabs when
their last selectable pane closes; unrelated panes in shared tabs stay open.
Stacked targets become active and expanded. Navigation hides floating panes for tiled targets and
shows them for floating targets; selecting an already-focused pane succeeds. The **󰋚 history toggle**
in the toolbar shows or hides saved conversations across all instances; history is hidden by default.
The toggle supports mouse clicks and keyboard focus/activation and appears when the integration is enabled.
Workspace matching includes repository
subdirectories and selects the closest owning workspace.

The top **Sessions** tab groups conversations beneath instance rows and outside-Tandem
directory groups. The tabs are **Sessions**, **Events**, **Rules**, **Providers**, and **Instances**.
**Instances** shows the template and instance tree. With OpenCode disabled, the tabs are
**Instances**, **Events**, **Rules**, and **Providers**. Click a tab or
press `[` / `]` from any main-view control to switch left / right. Instance/session switches retain
control focus; entering Events or Providers focuses its DataView.
The tab header stays visually active; dialogs and action menus own their keyboard input.
The **󰈈 show-all toggle** (`Shift+A`) reveals inactive instances, empty templates, and known
OpenCode folders without open clients. It is off at startup and resets to off with `Shift+H`.
The Sessions tab orders groups as externals with attached clients, instances with attached clients,
instances without attached clients, then externals without attached clients. Running instances
without OpenCode sessions appear when show-all is enabled. Known folders come from conversations,
clients, and local server directory receipts; the observer remembers them while the integration remains enabled.
Instance rows show the name, health/status, and a muted template
name, with the description on the second line. Conversation children show their title and activity
timer, then the latest question with its activity indicator. Search includes template and instance names.
History (`Shift+O`) controls saved and detached conversation children independently of folder
visibility; with history off, only attached children appear. Select an inactive folder and press `n`
to launch a new client. Both tabs retain creating and starting instances even before containers or
OpenCode clients are observed; with show-all off, inactive instances stay hidden. The toolbar actions,
filter values, and completion-sound setting are shared across both tabs. Each tab retains its own
expansion, selection, scroll position, and search. Inventory changes update both tabs; a removed
selection moves to a surviving row in that tab when one is available.
Clients without a conversation appear beneath their instance or external directory.
The first toolbar toggle, **󰕾 completion sound** (`Shift+N`), plays the desktop completion sound when a freshly observed
conversation changes from busy to idle. It is off by default and appears when the integration is
enabled on toolbars at least 50 columns wide; its hotkey remains active at narrower widths. The
desktop audio service controls playback. All toolbar toggles support mouse and keyboard activation.

A freshly observed conversation changing from busy to idle or awaiting an answer shows a `┃`
marker spanning its lines in the far-left gutter. The marker and row background pulse twice
over 600 ms. The marker then rises to full success color over 150 ms and fades into the
background over the configured fade duration (20 seconds by default) before disappearing.
Text positions, selection, and text colors stay intact. Initial, stale, and filter-only observations do not trigger these effects;
disabling animations suppresses them.
With the tree focused in Sessions or Instances, `Shift+J` / `Shift+K` selects the next / previous
busy conversation or conversation with an active completion marker in tree order, expanding its
ancestors and scrolling it into view. A successful jump clears a committed search; typing in search
keeps these keys as text.
Navigation stops at either end and leaves the tree unchanged when no eligible target exists.
Active markers retain their remaining lifetime when switching tabs.

Open panel creates a stacked pane in an observed tab for the instance or external directory, or
creates a named tab when there is no observed destination. It attaches to the
conversation's running local server without sending a prompt. An unavailable known server produces
an error; start that server before retrying.

Discovery uses companion receipts in `$XDG_STATE_HOME/tandem/opencode` (default
`~/.local/state/tandem/opencode`) and station daemon `port`/`dirs/*.dir` receipts under
`$OC_DAEMON_STATE` (default `$XDG_STATE_HOME/opencode-daemon`). Directory markers identify which
server serves each workspace; a cached directory alone does not count as a live conversation.
The companion publishes once per second; Tandem polls about every two seconds while its TUI runs.
Receipts require a live client PID and expire after ten seconds. The independently installed
companion continues reporting while Tandem's integration is disabled.

HTTP access is loopback-only, with redirects and proxies disabled. Credentials come from private
station password files, matching native `service.json` registration, or `OPENCODE_PASSWORD`
(V1: `OPENCODE_SERVER_PASSWORD` and optionally `OPENCODE_SERVER_USERNAME`). Credential files
must be regular, current-user-owned, and inaccessible to group/other users. Presence receipts
contain no credentials. V2 station resumptions use `opencode-station connect` to resolve the
password inside the new pane without putting it in Zellij's command arguments.
History requests the 21 most recent root conversations for each known directory so Tandem can
detect when its 20-row display window is full; busy conversations are fetched separately when
needed. Subagent conversations are excluded from the server inventory. Unavailable observations
and clients without the companion show an incomplete-observation label and details instead of
trustworthy counts. Confirmed deleted conversations are removed on refresh, including when a
client still reports the deleted conversation.
Refresh failures retain cached rows with stale markers
and do not generate recurring notifications. Cold standalone clients can report their displayed
conversation through the companion; detached work and history require a discoverable local server.

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
