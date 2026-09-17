# Tandem architecture

Tandem is a keyboard-first terminal application that exposes the same domain capabilities through a TUI and an MCP server.

## Boundaries

- `main.rs` installs diagnostics and delegates to `cli`.
- `cli.rs` selects TUI, stdio MCP, loopback HTTP MCP, or combined development mode.
- `lib.rs` initializes one `AppService` per process and wires it to each entry point.
- `service.rs` owns domain operations, persistence, external clients, synchronization, background work, errors, and notifications.
- `environments/` implements template discovery, Compose rendering, Docker inspection, gateway provisioning, readiness, and scoped lifecycle operations for `AppService` only.
- `store/environments/` defines domain snapshots, template manifests, operation outcomes, and naming validation.
- `app.rs` builds presentation-only `tuicore` nodes. Nodes read service snapshots and request operations through the service.
- `mcp.rs` defines typed MCP tools. Tools adapt input and output only, then call `AppService`.

## Invariants

1. The TUI and MCP surface use the same `AppService` contract.
2. Rendering performs no I/O, blocking work, or domain mutation.
3. Blocking work runs outside the TUI event loop and async MCP executor.
4. MCP HTTP binds only to loopback and validates loopback Host headers and same-origin browser requests. Remote transport requires authentication design and review.
5. Terminal lifecycle, themes, focus, and shared component keys come from `tuicore`.
6. Domain state and validation live by capability under `store/<domain>/`; domain I/O is requested through `AppService`.
7. App-specific keys are configurable and displayed from their resolved `KeySpec` labels.
8. Docker labels and runtime state own instance inventory. Templates and workspaces are filesystem resources; operation history and cached snapshots are process-local.
9. The TUI refreshes runtime inventory asynchronously every ten seconds while terminal-focused and every five minutes while unfocused, with an immediate request on focus gain. Terminals without focus reporting retain the focused cadence. Templates load at startup and manual refresh. Service mutations publish scoped SQLite revision counters shared across processes using the same Tandem home; an activated TUI observer checks these once per second and refreshes only affected domains. Runtime counters are namespace-scoped. The serial refresh worker coalesces requests and observes mutations that arrive during a refresh; MCP-only processes publish without activating an observer. Local UI state is read every 250 ms. Rendering performs no subprocess or filesystem work.
10. Instance mutations and gateway provisioning use advisory locks beneath the shared Tandem home. Stop preserves source workspaces and volumes.
    Instance and service restarts share the instance lock and verify Docker ownership before restarting existing containers. Instance restart excludes one-shot jobs; service restart selects exact service names within the instance. Both preserve configuration and data and leave the gateway untouched. Completion requires the original targeted containers to run and pass configured healthchecks and gateway content assertions within a ten-minute budget. Routed targets use a snapshot of current template readiness configuration under a shared template lock; targets without healthchecks or routes are verified only as running. The TUI emits one terminal success or failure notification per restart.
    Service start/stop selects an exact long-running service and uses the same ownership checks and instance lock. Both preserve existing containers and data, leaving dependencies and the gateway untouched. Start uses the restart readiness contract; stop requires no template and has a one-minute budget. Both emit one terminal success or failure notification.
    Instance starts hold shared template locks; template mutations hold exclusive locks. Distinct instances can start concurrently, and shared gateway provisioning waits for its exclusive lock within the startup deadline. Template deletion preflights ownership and instance locks, stops all owned containers, removes instance resources and workspaces, then removes template files. Cleanup receipts preserve ownership across failed starts and partial deletion; Docker labels remain the runtime inventory authority.
11. The gateway is a long-lived Traefik container on a shared external ingress network, with a loopback-only published port. Routes are generated from instance/service labels, never a route registry.
12. HTTP routes use `/<instance>/<service>/`; readiness checks assert configured content through the gateway, with bounded waits. Template authors own prefix-aware application configuration.
13. Agent guidance is authored in `agent-instructions.md`, seeded to an editable runtime Markdown file and read through `AppService` on each `get_instructions` call. Responses include a `manifest_schema` generated from the Rust manifest types independently of the user-owned Markdown. Manifest updates pass through `AppService`, require approval, validate before atomic replacement under the template lock, and perform no Docker/Git work; Compose references are checked at startup.
14. Resource polling begins after container discovery with a one-minute interval per process. Between periodic samples, new container identities/start times trigger a batch scoped to those containers while preserving other cached usage and the periodic deadline. Manual refreshes sample all active containers immediately. Manual refreshes and missing CPU baselines trigger two one-shot readings separated by a one-second pause; memory is published after the first reading. The in-flight guard covers both readings, and manual completion waits for sampling. Template scan errors do not block resource sampling; runtime discovery errors do. Each reading has a ten-second budget and up to eight concurrent requests through the selected local Docker Unix socket. Sampling failures are isolated per container and retain its last valid sample; created and restarting containers are excluded from sampling and totals. Memory uses Docker's working-set calculation; CPU percentages use consecutive counters scoped to container ID and start time. CPU remains unknown until a valid baseline exists. Service usage aggregates into instance and template totals.
    Pending instances and active in-process startup jobs are excluded from sampling and template totals; their snapshot usage stays unknown until startup finishes.
15. Docker inspection supplies explicit memory limits. Memory colors use explicit caps, and aggregate colors require caps for every active service. Searchable headerless property/value DataViews present metadata and details with semantic theme colors. Tree rows are two lines high, with memory above CPU in the resource column. Templates start expanded and instances collapsed; New instance resolves the template from any selected row. Template secondary lines show instance counts and, when available, the average of the latest five recorded successful startups in muted text; the same average supplies instance countdown estimates.
16. Workspace opening and saved-command execution read the persisted open command through the settings worker. Empty commands use the system folder opener; trusted custom commands run asynchronously through `sh -c` in the workspace with `TANDEM_INSTANCE` and `TANDEM_WORKSPACE` set as environment data. TUI and MCP saves share the service, and MCP command execution requires user approval. Child process streams stay detached from the terminal and MCP protocol.
17. Manifest `repositories` opt into core-managed provisioning before Compose configuration under the instance lock and startup deadline. Host Git clones into temporary sibling directories and installs complete checkouts with Linux no-replace rename. Branch instances selects the remote instance branch or creates a local branch from the remote default. Existing checkout roots and exact origins are validated without fetching or changing user work. A workspace ownership receipt binds provisioning to its template. Template authors own sources, targets, host credential access and app setup; undeclared templates keep their own checkout workflows. Git prompts, repository hooks and recursive submodule initialization are disabled.

## Growth shape

```text
src/
  app.rs
  cli.rs
  diagnostics.rs
  mcp.rs
  service.rs
  environments/
  store/<domain>/
  pages/<page>/
  components/
```

Keep Tandem as one package until another client needs its domain engine, build times create pressure, or a real platform/security boundary requires a crate split.
