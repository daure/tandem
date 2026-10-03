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
- `events_http.rs` adapts authenticated provider ingestion and feedback to service calls on a listener separate from MCP.
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
- Successfully prepared service instances with no containers retain their service execution kind and
  launch topology in SQLite. Their inventory reports Not started; fresh Docker observations take precedence.
- SQLite owns local instance identity and ownership, preparation outcomes, launch snapshots, lifecycle receipts,
  retained activities, startup requests/outcomes, settings, timing history, and cross-process refresh revisions.
  Records are keyed by namespace and case-normalized instance name; state writes publish revisions in the same transaction.
  Initialization enables write-ahead logging so read snapshots coexist with runtime writes.
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

Resource collection runs on a five-second cadence independently of the inventory schedule and terminal focus. Each batch publishes host memory and CPU temperature atomically with container usage. The TUI header incorporates the latest OpenCode usage only when a resource batch publishes; activity and inventory projections retain their own cadence. Initial CPU warm-up and manual refresh may publish sooner.

Long-running mutations publish progress through operation records. Completion is based on fresh observations of the targeted resources, not command exit alone. External commands run with bounded lifetimes and keep their output away from terminal and MCP protocol streams.

Instance startup runs through `AppService` in a detached invocation of the same executable, with its
own terminal session and disconnected standard streams. The initiating process persists the request
and transfers its instance lock and operation-specific lease through inherited descriptors. The worker
holds both through readiness and final outcome publication; its external commands cannot inherit them.
Readers combine durable startup records with fresh runtime observations, including before containers
exist. Losing the lease or exceeding the deadline surfaces interruption; observing state performs no
automatic retry. Other lifecycle actions retain their existing execution scope.

## OpenCode observation

`service/opencode` owns a cancellable, single-flight event-driven observer and navigation tasks, gated by the
persisted `integrations.opencode` setting. `environments/opencode` reads client-presence and
station-daemon receipts, queries loopback OpenCode HTTP endpoints, and invokes bounded Zellij
commands. `store/opencode` owns session state, overlapping counts, and workspace matching;
the TUI projects these snapshots into instance summaries, an outside-Tandem workspace group,
and conversation/pane children.

Filesystem notifications discover receipt changes and servers. One authenticated live-event
subscription per local server requests coalesced reconciliation reads; startup and reconnect request
a baseline because subscriptions have no replay. Client-local receipt changes update attachment,
metadata, and tab positions without server reads. Unchanged heartbeats leave the snapshot untouched.
Stream failures mark affected state stale and retry with bounded backoff. Unverifiable conversations
and clients stay hidden during three verification checks, spaced by one and two seconds. Exhaustion
removes their active tracking and writes one diagnostic entry per source. Meaningful receipt changes
or newly listening daemon sources allow rediscovery; heartbeats and server events do not reset an
active retry budget. Known unfinished work in definitely missing folders remains visible with warning
glyphs, while detached confirmed-idle history in those folders stays hidden. Existing clients remain
navigable and closable through fresh receipt and live-pane verification; creating or resuming a client
requires an existing directory. Scoped observation warnings accompany visible affected work, while
unscoped infrastructure failures remain explicit. The five-second process-sampling timer also checks
client liveness, presence expiry, and Zellij tab positions without server reads. Open Sessions workspace groups follow
their earliest attached Zellij tab within each ownership group, with stable fallback ordering when
positions are unavailable. Failed tab queries retain the last trustworthy positions. Watchers and
subscriptions are disposed when the integration is disabled.
Daemon port records require an IPv4 loopback listener or a fresh client receipt before observation.
Saved idle history alone does not trigger requests to stopped servers. The liveness check discovers listener
changes, so retained records become observable when their server starts and release subscriptions
when it stops without dependent work. Dormant records remain owned by their launcher.
Observation skips definitely missing directories without fresh clients or unfinished work, even
when their shared server listens. Filesystem access errors retain uncertainty. Failed directory
status reads preserve cached conversations subject to verification retention.

The bundled OpenCode TUI companion runs in each client and publishes its current route and open
session tabs with PID, heartbeat, server, and Zellij identity. Native reactive computations track
routes, tab order, and cached session metadata; server-event listeners cover cache changes. All open
tabs attach to the same client pane and retain their native positions; navigation selects the requested
OpenCode tab before focusing that pane. Server processes are shared
across directories and do not own client attachment identity. Fresh receipts and live panes
establish attachment and authorize navigation or closure of that exact pane. Observed conversations
can be resumed in their recorded directory on their loopback server, including external workspaces.
New clients target an instance workspace or an observed directory. Prepared instance workspaces receive
validated guidance preparation, while external directories remain outside Tandem provisioning.
The TUI's new-session action reuses a V2 client through an authenticated loopback companion endpoint
advertised in its private presence receipt. The companion focuses the first open idle tab in native
order whose directory matches and whose synchronized messages, queued input, and forms are empty,
or creates a session tab when none qualifies. Fresh receipts and live Zellij inventory authorize that
exact client. Failed requests surface uncertain
creation without launching a second client. Clientless workspaces and V1 clients use pane launching.
During instance creation, the detached startup worker validates the workspace and clears history,
copies template seed files, and generates workspace guidance before launching a requested client.
The client launches before repository provisioning and Compose rendering. Early guidance uses declared
repositories and previewed services; preparation refreshes verified mappings before container startup
only if the file created by that startup remains unchanged. Existing guidance and client edits are preserved.
The worker retains the launch outcome separately from instance readiness; launch failures permit
provisioning to continue. Seed-copying or guidance-generation failures block the requested launch.
Creation requests persist a default-on service-start flag. Prepare-only requests complete workspace,
repository and Compose preparation while skipping Compose up, gateway startup and readiness waits.
New instance sessions receive state-aware guidance before initial input; session opening leaves container
state unchanged. V2 uses durable session instruction entries. V1 appends synthetic no-reply context.
Rule launches set their model/variant at session creation and send task input after attaching guidance.
Bulk closure selects observed panes by instance ownership or exact external directory and excludes
owned panes from the external aggregate. With integration enabled, instance purge closes associated
clients under the instance lock after ownership validation and before resource deletion. Verified local
ownership records allow client closure before Docker inspection. Group pane closures run concurrently;
fresh receipts authorize exact-pane closure and live pane inventory verifies completion. Closure
failures preserve that instance's resources. Version-specific server status and directory-scoped forms/questions
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
template assertions. Startup builds services that declare Compose build configurations with Docker's layer
cache before starting containers; instance-name reuse can retain image tags from a previous template.
Verified inventory and instance cleanup do not require the template files to exist.
MCP initialization directs clients to bundled placement rules alongside preserved editable guidance;
upgrades distribute the baseline without replacing user-owned instructions.

Optional `tandem-files/` contents seed the workspace root before repository provisioning. Preparation
preserves existing regular files and rejects symlinks, special files, reserved workspace paths, and
repository-target overlap. Template snapshots include the recursive file tree and metadata; the
background template observer detects additions, edits, removals, and folder deletion for every interface.

The shared template repository can contain `instances/`, `providers/`, and `rules/` catalogs. Instance discovery
uses `instances/` when present; flat catalogs are supported without relocation, and mixed instance
layouts fail initialization. MCP guidance reports the shared repository and effective catalog paths.
Writable-mount protection covers the entire shared repository, including sibling provider packages.

## Provider events

`store/events` defines versioned envelopes, four typed profiles, processing attempts, and feedback
messages. `environments/events` owns namespace/provider-scoped SQLite identities and transactional
ingestion, attempts, and notification queues in the private application database. Receipt creates
an event, pending attempt, and `received` notification atomically. Matching redeliveries retain their
receipt; conflicting content rejects the batch. Provider credentials authorize only their own
ingestion and feedback, independently of event-supplied metadata.
System-event severity and optional environment/color hints retain source values; a pure projection
selects the semantic tone from an explicit hint or an exact common label. Missing optional fields are
omitted from serialization so canonical retained events remain compatible with duplicate redelivery.

`service/events` schedules blocking database work and single-flight feed observation. Every TUI
process reads durable events through background snapshots, including when a separate sidecar owns
the listener. Failed observations retain prior data with error provenance. The event DataView and
profile renderers are pure presentation; the root coordinator retains modal/focus routing while
capability-specific event logic lives in `app/events`.

Confirmed event deletion removes namespace-scoped events, processing history, pending rule work,
and associated feedback atomically under the rule-worker lease. Provisioning or launching actions
block deletion. Instances, sessions, provider credentials, and rule definitions are preserved.
Dispatch admission history is namespace-owned and survives event deletion to enforce rate limits.

Processing attempts are pending or accepted. Acceptance means a rule matched and durably triggered
an independent action. Explicit replay of any retained event uses current enabled rules with a separate,
idempotent attempt identity. Retained processing history preserves identities and timestamps through
atomic schema migration. Feedback is `received`, `replayed`, or `assigned`; retained `acknowledged`
notifications remain readable. Providers
poll and acknowledge notifications independently of event acceptance. Durable records survive
sidecar/provider disconnects; external provider side effects require idempotency or reconciliation.
Agent assignment, session launch, prompt delivery, and task completion remain separate facts.

`store/rules` defines versioned rule snapshots, bounded boolean-only Rhai predicates, in-memory
Handlebars prompt rendering, and independent per-rule/per-attempt acceptances. Scripts expose no
host I/O. Prompt templates use strict field interpolation and literal event text without HTML
escaping. Rendering bounds output to 64 KiB, nesting to 32 levels, and template evaluations to 50,000;
budgeted block execution also bounds empty loops and inline-partial recursion.
`environments/rules` owns shared definitions in `templates/rules/<name>/rule.json` and namespace-local
activation, Zellij targets, optimistic revisions, cached definitions, pinned evaluations, acceptances,
dispatch admission history, and assignment feedback in SQLite. A shared catalog lock precedes write
transactions for rule saves, discovery, receipt, and replay. New files are inactive; external definition
edits invalidate local activation. File removal hides the rule and disables future matches, retaining
its revision identity and pinned history. Invalid files fail closed. Receipt/replay transactions reconcile
the catalog before selecting rules and
pin every enabled rule revision; completed evaluations survive redelivery and recovery. Match errors
and false results are separate outcomes. A match atomically creates its acceptance and assigned
instance identity before external actions, regardless of sibling outcomes.

`service/rules` owns authorization, previews, recovery, and dispatch.
TUI activation captures its process's current Zellij session; MCP can supply an explicit live target.
Activation verifies the target. Dispatch rechecks it before preparation and before its durable launch
marker. A missing target is a pre-launch failure requiring reactivation; accepted revisions retain their
pinned destination, including across retries.
A private namespace file lease serializes background cycles across TUI and owned event-sidecar
processes. Admission permits four
active actions and ten starts per minute, counting retries. Each action provisions a fresh instance
through the existing detached startup contract. Successful preparation and requested service readiness
gate model/prompt launch; each pinned rule revision owns its default-on service-start flag.
The launch marker is durable before contacting Zellij; an interrupted or uncertain launch is surfaced
without automatic resend. Observed exact workspace/pane linkage records the session and `launched`
outcome, which does not certify prompt delivery or task completion. Confirmed pre-launch retries keep
the assigned instance; replay intentionally creates new actions. Assignment and its originating-provider
notification commit together. The sidecar polls automation independently of open UI/MCP clients.

`app/rules` projects two-row rule/acceptance DataViews and editable bottom dialogs through tuicore.
Event details retain all acceptances; rule histories filter by rule identity. Exact history navigation
loads retained events beyond the feed and clears conflicting filters. MCP shares the same service
contract for definitions, previews, event inspection, replay, and restricted retries.

Developer provider packages have independent Docker build contexts, read-only private token mounts,
and named checkpoint volumes. Their sample Python runtime is a fixture dependency; the provider
contract is language-independent HTTP. Provider containers have lifecycle ownership separate from
application instances. The sidecar defaults to loopback, rejects browser origins, and exposes no MCP
transport. Events are retained durably while the feed and attempt-history projections are bounded.

`store/providers` owns manifest, action, and runtime projections. `environments/providers` discovers
packages, snapshots validated launches in SQLite, materializes private Compose files, and operates
only positively owned collectors and volumes. Namespace-scoped locks serialize conflicting work;
fresh Docker evidence verifies lifecycle completion. Stopping preserves checkpoints and source
identities. Launch snapshots preserve source identity and permit lifecycle inspection after package
deletion. `service/providers` coordinates bounded blocking operations and snapshot observation;
`app/providers`, CLI, and MCP adapt that contract without owning lifecycle I/O.
Lifecycle completion publishes the targeted collector's verified state and clears its operation
before any full-inventory refresh. Observation revisions prevent an older in-flight snapshot from
overwriting a completed action. The background observer owns full provider discovery and refresh.

Provider manifests optionally declare streams and cooperative stream-control capability. Namespace/source/
stream-scoped SQLite records own desired collection state and monotonic control revisions. The authenticated
sidecar exposes only the credential's controls and accepts acknowledgments of the current exact revision
and desired state. Collectors acknowledge after applying collection changes and refresh that evidence;
five-second expiry leaves uncertain collection explicit. Stream lifecycle serializes with collector lifecycle,
waits up to ten seconds for acknowledgment, and rechecks the owned collector before reporting completion.
Collector startup enables all declared streams and invalidates prior evidence; collector Stop invalidates
stream evidence before Docker work. Stream Stop blocks ingestion as a race guard, but successful completion
requires the collector to cancel its stream work and discard buffered events. Resume is live-only; source
checkpoint recovery and backfill are outside the contract. Sibling streams and the control loop keep running.
An individual stream start provisions or resumes a stopped collector with only that stream enabled;
running-collector changes preserve siblings. Stopping the last enabled stream stops the collector under
the same lifecycle lock. Stream completion publishes verified collector and stream states together.
Durable control-request timestamps project enabled streams as Starting for up to ten seconds while awaiting
their first acknowledgment. Errors, timeouts and expired acknowledgments remain unverified; readiness does
not depend on source events.
The Providers DataView projects provider parents and stream children with stable identities, independent
details, scoped menus, retained-history counts, and exact provider/stream event links. Observed streams without
declared control capability remain inspectable without claiming collection state or offering lifecycle actions.

Provider lifecycle persists a namespace/source-scoped ingestion gate before Docker work. Stop
disables ingestion; authenticated batches receive discarded IDs without events, attempts, or
feedback. The ingestion transaction reads the gate so detached sidecars share the same cutoff.
Failed Stop operations keep ingestion disabled; unavailable actions restore the prior gate.
Start enables ingestion before collector work and restores the prior gate on failure.

Provider lifecycle offers Start and Stop. Start prepares its private token and ensures a detached
native sidecar. It unpauses owned paused collectors and verifies startup is running and unpaused.
The sidecar worker owns a namespace lease, binds loopback, and
publishes a private receipt with an independently probed process identity. Its recorded address survives
restart. The TUI supervises it for active collectors; Start and rule activation also ensure readiness.
On Linux, readiness compares the receiver's executable device/inode with the installed executable.
Replacement holds the namespace startup gate, pins the verified process with a pidfd, rechecks its
receipt identity, requests graceful shutdown and waits for lease release before binding the retained
address. Open clients whose executable was unlinked resolve its installed path for receiver startup
and freshness checks. Unverifiable ownership or incomplete shutdown blocks replacement.
Sidecar and collector lifetimes are independent of open terminal clients. MCP remains a separate
transport. An optional foreground `serve-events` mode supports
protocol tests. Providers is the fourth tab, with lifecycle approval, bounded logs, and exact source
links to the second Events tab.

Provider deletion is an explicitly confirmed MCP service operation. An exclusive package lock excludes
cross-namespace lifecycle work; namespace/provider and source locks exclude collection changes and
identity reuse. The rule-worker lease gates dispatch admission. Durable deletion markers block queued
dispatch and collection restarts across failures and process restarts. Positive Docker labels authorize
collector/checkpoint removal; canonical package/runtime/credential paths bound filesystem cleanup.
Instance purge rechecks the originating startup identity under the instance lock before closing clients,
deleting workspace OpenCode history and removing resources. External failures retain launch/event
identity for retry. Successful cleanup atomically removes provider credentials, streams, events and
dependent history, launch records and the deletion marker. Shared rules, sidecars, namespace admission
history and Docker caches remain independently owned.

## Growth rules

Keep Tandem as one package until a real boundary appears: another client needs the domain engine, build pressure warrants separation, or deployment/security requires an independent process. Split modules by capability before splitting crates. New infrastructure integrations sit behind `AppService`; new interfaces remain adapters over it.
