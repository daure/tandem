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
- Runtime journals own workspace-only identity, preparation outcomes, launch topology, and recoverable activity evidence.
- The filesystem owns templates, generated configuration, repositories, and workspaces.
- SQLite owns settings, startup history, and cross-process refresh revisions.
- Operation history, resource samples, and transient UI state are process-local caches.

An empty or failed Docker observation never proves that an instance is workspace-only. Recovery and cleanup require positive ownership evidence from the appropriate source.

## Background work and refresh

A serial refresh worker coalesces inventory requests and uses persisted revision counters to observe changes from other Tandem processes. Resource collection augments inventory snapshots but does not determine lifecycle or health. Sampling failures retain valid prior readings with error provenance.

Long-running mutations publish progress through operation records. Completion is based on fresh observations of the targeted resources, not command exit alone. External commands run with bounded lifetimes and keep their output away from terminal and MCP protocol streams.

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
establish attachment and authorize navigation or closure of that exact pane; attaching a detached
conversation requires an owning Tandem instance. Server session status establishes activity. Titles are presentation data.
Unknown observations stay explicit, and disabling the integration discards its cache and cancels
its work without changing externally owned servers or panes. Companion installation is an
explicit CLI operation; user-owned OpenCode configuration is preserved.

## Template and workspace model

A template may describe a Compose environment, a repository-backed workspace, a guidance-only setup, or a blank workspace identified by an empty `tandem.json` object. New templates contain only that manifest. Instance execution kind is fixed when prepared because container-backed and workspace-only instances have different ownership evidence and lifecycle behavior.

Template manifests declare core-managed repositories, routes, and setup jobs. Template authors own application-specific setup and prefix-aware routing. Tandem owns safe provisioning, generated workspace guidance, and lifecycle coordination.

## Growth rules

Keep Tandem as one package until a real boundary appears: another client needs the domain engine, build pressure warrants separation, or deployment/security requires an independent process. Split modules by capability before splitting crates. New infrastructure integrations sit behind `AppService`; new interfaces remain adapters over it.
