# Tandem architecture

Tandem is a keyboard-first terminal application that exposes one domain through TUI, CLI, and MCP interfaces. It remains a single Rust package while those interfaces share the same application and domain boundaries.

## System shape

```text
TUI (`app`)      CLI (`cli`)      MCP (`mcp`)
       \             |             /
                    AppService
                        |
          domain state and environment adapters
                        |
        Docker · filesystem · Git · SQLite · HTTP · processes
```

Presentation layers adapt input and output. `AppService` is the sole application boundary for domain operations, persistence, external clients, background work, and notifications. Environment modules perform infrastructure I/O for the service; store modules define domain state and pure projections.

## Module responsibilities

- `main.rs` installs diagnostics and enters the selected interface.
- `lib.rs` initializes one `AppService` per process and wires long-running interfaces.
- `cli.rs`, `mcp.rs`, and `app.rs` translate their transports into service calls. They contain no domain rules or infrastructure I/O.
- `service.rs` and `service/` coordinate use cases, workers, persistence, and external adapters.
- `environments/` owns Docker, Compose, gateway, repository, filesystem, and process integration.
- `store/<domain>/` owns domain types, validation, state transitions, and pure summaries.
- `app/` contains presentation-only TUI state, composition, and projections.

## Architectural invariants

1. **One application contract.** TUI, CLI, and MCP behavior converges in `AppService`; adding an interface must not create a second domain path.
2. **Pure presentation.** TUI render and projection paths perform no I/O, blocking work, or domain mutation. `tuicore` owns terminal lifecycle, layout, focus, themes, and configurable keybindings.
3. **Explicit blocking boundaries.** Docker, Git, filesystem, database, HTTP, and child-process work runs outside the TUI event loop and async MCP executor.
4. **Transport isolation.** MCP stdio reserves stdout for protocol data. HTTP MCP remains loopback-only unless a separate authenticated remote-access design is approved.
5. **Typed domain state.** Raw runtime observations, interpreted status, operation outcomes, and resource samples remain distinct types. Shared projections classify status for every interface.
6. **Ownership before mutation.** Paths, Docker resources, repositories, and persisted records are validated against Tandem ownership before destructive or state-changing work.
7. **Serialized conflicting work.** Per-instance locks protect lifecycle changes; template and gateway locks protect shared resources. Unrelated instances may proceed concurrently.
8. **Snapshots for readers.** Interfaces consume snapshots rather than reaching into adapters. Background workers publish completed observations and retain the last trustworthy state when an external dependency fails.

## State ownership

- Docker labels and inspected containers are authoritative for container-backed runtime inventory.
- SQLite owns local instance identity and ownership, preparation outcomes, launch snapshots, lifecycle receipts,
  retained activities, startup requests/outcomes, settings, timing history, and cross-process refresh revisions.
  Records are keyed by namespace and case-normalized instance name; state writes publish revisions in the same transaction.
- The filesystem owns team-shareable templates, repositories, workspaces, locks, and private runtime artifacts.
  Resolved Compose snapshots are materialized atomically beneath `runtime/<namespace>/<instance>/compose.json`
  with private permissions. SQLite snapshots repair altered or missing regular materializations; unsafe paths fail closed.
- The latest startup request, bounded progress, and outcome per instance are durable runtime records.
- Other operation history, resource samples, and transient UI state are process-local caches.

An empty or failed Docker observation never proves that an instance is workspace-only. Recovery and cleanup require positive ownership evidence from the appropriate source.

Ordinary service initialization imports validated legacy records before starting observers or workers.
Import holds a migration gate and legacy operation locks, backs up originals privately, and commits records
and its completion marker atomically. Conflicts block initialization. Imported-file cleanup is retryable;
SQL remains authoritative after import. Old clients must be closed during the upgrade because they do not
participate in the migration gate. Detached workers require their parent's completed import marker.

## Background work and refresh

A serial refresh worker coalesces inventory requests and uses persisted revision counters to observe changes from other Tandem processes. Resource collection augments inventory snapshots but does not determine lifecycle or health. Sampling failures retain valid prior readings with error provenance.

Long-running mutations publish progress through operation records. Completion is based on fresh observations of the targeted resources, not command exit alone. External commands run with bounded lifetimes and keep their output away from terminal and MCP protocol streams.

Instance startup runs through `AppService` in a detached invocation of the same executable, with its
own terminal session and disconnected standard streams. The initiating process persists the request
and transfers its instance lock and operation-specific lease through inherited descriptors. The worker
holds both through readiness and final outcome publication; its external commands cannot inherit them.
Readers combine durable startup records with fresh runtime observations, including before containers
exist. Losing the lease or exceeding the deadline surfaces interruption; observing state performs no
automatic retry. Other lifecycle actions retain their existing execution scope.

## OpenCode observation

`service/opencode` owns a cancellable, single-flight observer and navigation tasks, gated by the
persisted `integrations.opencode` setting. `environments/opencode` reads client-presence and
station-daemon receipts, queries loopback OpenCode HTTP endpoints, and invokes bounded Zellij
commands. `store/opencode` owns session state, overlapping counts, and workspace matching;
the TUI projects these snapshots into instance summaries, an outside-Tandem workspace group,
and conversation/pane children.

The bundled OpenCode TUI companion runs in each client and publishes its current route's
conversation ID with PID, heartbeat, server, and Zellij identity. Server processes are shared
across directories and do not own client attachment identity. Fresh receipts and live panes
establish attachment and authorize navigation or closure of that exact pane. Observed conversations
can be resumed in their recorded directory on their loopback server, including external workspaces.
New clients target an instance workspace or an observed directory. Prepared instance workspaces receive
validated guidance preparation, while external directories remain outside Tandem provisioning.
During instance creation, the detached startup worker validates the workspace and clears history,
copies template seed files, and generates workspace guidance before launching a requested client.
The client launches before repository provisioning and Compose rendering. Early guidance uses declared
repositories and previewed services; preparation refreshes verified mappings before container startup
only if the file created by that startup remains unchanged. Existing guidance and client edits are preserved.
The worker retains the launch outcome separately from instance readiness; launch failures permit
provisioning to continue. Seed-copying or guidance-generation failures block the requested launch.
Bulk closure selects observed panes by instance ownership or exact external directory and excludes
owned panes from the external aggregate. With integration enabled, instance purge closes associated
clients under the instance lock after ownership validation and before resource deletion. Verified local
ownership records allow client closure before Docker inspection. Group pane closures run concurrently;
fresh receipts authorize exact-pane closure and live pane inventory verifies completion. Closure
failures preserve that instance's resources. Directory-scoped server status and pending question requests
establish activity. Awaiting an answer pauses busy timing and triggers completion feedback once on
the transition from busy. Titles are presentation data.
Unknown observations stay explicit, and disabling the integration discards its cache and cancels
its work without changing externally owned servers or panes. Companion installation is an
explicit CLI operation; user-owned OpenCode configuration is preserved.

The observer samples known client PIDs through Linux `/proc` on its blocking worker. CPU deltas
require matching process start times; missing processes are evicted and failed readings retain
stale provenance. Domain projections deduplicate PIDs and aggregate client usage by conversation,
pane, workspace, instance, and template. TUI totals combine these summaries with container usage
independently of the displayed tree. Shared-server and child-process costs are outside client scope.

## Template and workspace model

A template may describe a Compose environment, a repository-backed workspace, a guidance-only or files-only setup, or a blank workspace identified by an empty `tandem.json` object. New templates contain only that manifest. Instance execution kind is fixed when prepared because container-backed and workspace-only instances have different ownership evidence and lifecycle behavior.

Template manifests declare core-managed repositories, routes, and setup jobs. Template authors own application-specific setup and prefix-aware routing. Tandem owns safe provisioning, generated workspace guidance, and lifecycle coordination.

Templates contain shared recipes and static assets. Lifecycle operations write local instance state outside
templates. Compose resolves relative assets from the template directory; writable binds, bind-backed local
volumes, and local build-cache exports must not expose the templates root or its ancestors. Existing static
template assets may be mounted read-only. This validation is a configuration guardrail for trusted templates,
not a sandbox against privileged containers or external Docker resources.

Creation and instance startup apply the selected template. Existing-container restart and service start use
saved launch-time readiness assertions; imported launches without a historical manifest require current
template assertions. Verified inventory and instance cleanup do not require the template files to exist.
MCP initialization directs clients to bundled placement rules alongside preserved editable guidance;
upgrades distribute the baseline without replacing user-owned instructions.

Optional `tandem-files/` contents seed the workspace root before repository provisioning. Preparation
preserves existing regular files and rejects symlinks, special files, reserved workspace paths, and
repository-target overlap. Template snapshots include the recursive file tree and metadata; the
background template observer detects additions, edits, removals, and folder deletion for every interface.

## Growth rules

Keep Tandem as one package until a real boundary appears: another client needs the domain engine, build pressure warrants separation, or deployment/security requires an independent process. Split modules by capability before splitting crates. New infrastructure integrations sit behind `AppService`; new interfaces remain adapters over it.
