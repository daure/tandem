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
9. The TUI refreshes inventory asynchronously every two seconds and local progress every 250 ms. Rendering performs no subprocess or filesystem work.
10. Instance mutations and gateway provisioning use advisory locks beneath the shared Tandem home. Stop preserves source workspaces and volumes.
    Template creation, instance startup, and template deletion share a template lock. Template deletion preflights ownership and instance locks, stops all owned containers, removes instance resources and workspaces, then removes template files. Cleanup receipts preserve ownership across failed starts and partial deletion; Docker labels remain the runtime inventory authority.
11. The gateway is a long-lived Traefik container on a shared external ingress network, with a loopback-only published port. Routes are generated from instance/service labels, never a route registry.
12. HTTP routes use `/<instance>/<service>/`; readiness checks assert configured content through the gateway, with bounded waits. Template authors own prefix-aware application configuration.
13. Agent guidance is authored in `agent-instructions.md`, seeded to an editable runtime Markdown file and read through `AppService` on each `get_instructions` call.
14. Resource polling begins after container discovery and runs at most once per minute per process. A missing CPU baseline triggers two one-shot readings separated by a one-second pause; memory is published after the first reading. The in-flight guard covers both readings. Each reading has a ten-second budget and up to eight concurrent requests through the selected local Docker Unix socket. Sampling failures are isolated per container and retain its last valid sample; created and restarting containers are excluded from sampling and totals. Memory uses Docker's working-set calculation; CPU percentages use consecutive counters scoped to container ID and start time. CPU remains unknown until a valid baseline exists. Service usage aggregates into instance and template totals.
15. Docker inspection supplies explicit memory limits. Memory colors use explicit caps, and aggregate colors require caps for every active service. Searchable headerless property/value DataViews present metadata and details with semantic theme colors. Tree rows are two lines high, with memory above CPU in the resource column.
16. Workspace opening and saved-command execution read the persisted open command through the settings worker. Empty commands use the system folder opener; trusted custom commands run asynchronously through `sh -c` in the workspace with `TANDEM_INSTANCE` and `TANDEM_WORKSPACE` set as environment data. TUI and MCP saves share the service, and MCP command execution requires user approval. Child process streams stay detached from the terminal and MCP protocol.

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
