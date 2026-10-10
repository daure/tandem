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

`tandem mcp-instance` is a separate stdio adapter exposing instance-local Start, Stop, repository updates and Conclude through
`AppService`. Binding resolves its working directory against namespace-owned journal and ownership
records and holds an open workspace-directory descriptor. Each mutation compares the live directory's
device/inode with that pinned descriptor under the instance lock; lock ownership transfers through
the existing lifecycle workers, excluding deletion and name reuse through completion. The surface
also exposes a scoped readiness ping. It verifies the workspace under the instance lock and reads
the persisted ping toggle and sound selection before scheduling host playback. Ping leaves lifecycle
and report state unchanged; its receipt confirms scheduling rather than audible delivery. The ping
toggle defaults off and is shared by processes using the same Tandem home.
TUI startup persists a disabled ping gate before entering the event loop and initializes its local
completion and rule-acceptance gates disabled. Selected sounds remain saved; overview resets preserve
notification enablement. CLI and MCP startup preserve the shared gate.
Enabled pings with OpenCode session metadata publish per-session receipts in the instance journal.
Namespace-scoped refresh revisions carry these receipts to independent TUI processes; projection
matches both instance ownership and session identity. Completion markers use semantic success color,
while ping markers use the theme accent. An active ping marker retains its original fade timing
through activity changes and completion; completion sound remains independent.
The surface provides `get_instructions` with bound identity, bundled `instance-core-guidance.md`, and editable
`instance-instructions.md` seeded from `instance-agent-instructions.md`. Existing editable guidance
is preserved and reread per call. Initialization directs agents to read both fields before using tools
and ask the user to resolve conflicts. Guidance reads verify ownership and the pinned workspace before
and after reading, so they remain available during startup and reject workspace replacement.
Guidance replies include a read-only status snapshot using shared instance and service projections.
Docker observation has a two-second budget without waiting for the instance lifecycle lock; failures
preserve guidance and expose stale/error metadata. Recorded topology supplies configured services
without containers; topology completeness and observation time distinguish unknown state from stopped.
The surface has no caller-selected lifecycle target and is not a host-process sandbox. Preparation
seeds workspace configuration in syntax supported by OpenCode V1 and V2, with explicit Tandem settings,
preserving existing root and `.opencode` JSON/JSONC configuration. Configuring the surface grants
these lifecycle capabilities; guidance treats Conclude as instruction-only, independently of task
completion or cleanup needs. Stopping services remains independent of event-task completion. Conclude
controls any owned bound instance. A retained acceptance must uniquely match its template and originating
startup lineage; mismatched or ambiguous records fail closed. Linked conclusions commit an immutable
acceptance-owned report; unlinked conclusions skip report persistence. Both transfer the instance lock,
rule-worker lease and conclusion lease to a detached worker. Reportless workers use acceptance ID zero
internally and an instance-keyed conclusion lease; they verify the instance remains unlinked before purge.
The worker closes associated clients regardless of the observation setting and
purges owned resources through the existing deletion contract. Client closure may interrupt the MCP
reply; linked reports retain cleanup state and failure details, and a lost conclusion lease projects an
interrupted outcome. Unlinked outcomes use instance inspection and retained lifecycle activities.
Identical saved reports allow cleanup retry; different report contents conflict.
Reported acceptances retain dispatch history and are excluded from dispatch admission and retries.
Namespace-scoped report search matches any case-insensitive literal substring in title, summary or
Markdown and returns acceptance summaries; report retrieval uses the acceptance ID. Report storage
is independent of instance state and cascades only with acceptance/event/provider history deletion.
Management MCP exposes report search and retrieval through the same service contract. Rule snapshots
include report summaries and observed cleanup outcomes; Events and rule histories expose full-report reads.

Instance repository updates run on a blocking service worker under the pinned workspace's instance
lock. The current owned template supplies declared targets; each checkout requires a real contained
path and its exact declared origin. Explicit origin-upstream fetches and fast-forward-only merges
preserve local commits and report updated, current, skipped or failed outcomes with observed revisions.
Dirty, diverged, detached, untracked branches and active Git operations are skipped; failures remain
per repository. Git hooks, automatic stashing and submodule recursion are disabled, and merges protect
ignored local files. A ten-minute batch budget bounds one-minute repository operations; final revision
reads have separate five-second verification budgets. Shell sessions remain outside Tandem's lock.

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
- Startup records retain the originating operation ID across preparation retries, Start and authorized
  acceptance recreation. The acceptance keeps its original operation ID. Purge releases instance identity;
  unrelated name reuse receives a fresh origin. Records without an explicit origin use their retained
  operation ID; a conflicting historical acceptance fails closed rather than inferring ownership from names.
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
Purge can remove only retained metadata for failed cold startups with no prepared journal, ownership
or launch record after the startup lease is released. The instance lock excludes active work and name
reuse; unverified workspaces, runtime resources, clients and conversation history are preserved.

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
OpenCode tab before focusing that pane. Conversation closure uses authenticated companion tab control
for the selected V2 tab, including the last tab. Fresh receipts verify native tab removal before
closing an empty client pane, so closed tabs stay closed across client restarts;
sibling tabs and saved conversation history are preserved. V1 conversation closure targets its
client pane. Server processes are shared across directories and do not own client attachment identity.
Fresh receipts and live panes
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
Prompted conversations in existing instances use a dedicated CLI/management MCP service operation.
Admission validates retained ownership and the workspace under an instance lock held through delivery.
An authenticated companion creates a fresh session, attaches guidance, then submits literal task input.
Tab-capable clients are reused; clientless workspaces launch a client without input before requesting
the conversation. Receipts retain session/server identity and distinguish submitted, not-submitted and
uncertain task input. Lost responses never trigger fallback launches or automatic resend; confirmed
submission is server acceptance rather than model completion. Services, repositories and history are preserved.
Session-ID follow-up input resolves uniquely across known reachable V2 servers and verifies the owning
instance and real workspace containment under its instance lock. The companion reopens that exact
conversation in a tab; absent clients attach to its server and session before submission.
Omitted settings preserve session choices. Explicit model, variant and agent overrides persist before
input and may affect ongoing work's subsequent turns. Queue mode admits durable queued input without
interrupting work or answering forms; interrupt mode stops execution before sending steering input.
Abort checks active execution, forms and permissions before setup and before submission; concurrent
external input remains outside Tandem's lock. Receipts distinguish queue admission from submission,
unsent input and uncertain delivery. Setup failures preserve any applied settings or interruption.
CLI/management MCP session listings share a read-only service operation with an optional instance filter.
Results default to attached sessions and freshly observed busy/awaiting-answer work. The default-false
`include_closed` flag includes saved and unverified detached history through the same projection.
It reads namespace-scoped retained links independently of rule catalog synchronization, performs bounded
fresh observation on the service runtime, and
projects instance ownership by nearest workspace ancestry. Recent root history uses the observer's
per-directory window alongside known active and retained sessions. Listings expose attachment evidence,
unknown stale activity, missing workspaces, and partial discovery errors without launching clients or
starting servers. Concurrent local session edits and folder cleanup suppression apply before projection;
prompt admission independently verifies identity and ownership.
Rule launches set their model and optional variant at session creation and send task input after attaching guidance.
CLI and MCP creation requests carry optional model/variant selections through the detached worker;
V2 companion-created conversations capture the client's selected model and variant before creation
unless explicitly overridden. Variant-only requests retain the selected model; explicit models without
a variant use their model default. Launch selections are client-scoped and leave shared server configuration unchanged.
Bulk closure selects observed panes by instance ownership or exact external directory and excludes
owned panes from the external aggregate. With integration enabled, instance purge closes associated
clients under the instance lock after ownership validation and before resource deletion. Verified local
ownership records allow client closure before Docker inspection. Group pane closures run concurrently;
fresh receipts authorize exact-pane closure and live pane inventory verifies completion. Closure
failures preserve that instance's resources. Version-specific server status and directory-scoped forms/questions
establish activity. Awaiting an answer pauses busy timing and triggers completion feedback once on
the transition from busy. Titles are presentation data.
Session rename and confirmed deletion run through the service on authenticated local server APIs.
Deletion verifies exact session/directory identity and descendants, closes their verified client panes,
and interrupts execution before removing the selected conversation tree and checking its absence.
External-directory cleanup requires detached, idle history. Admission immediately suppresses the folder
in service snapshots and launches a detached worker independently of navigation. The worker validates
exact-directory ancestry and activity, deletes verified top-level conversation trees, removes matching
daemon directory receipts, and persists removed conversation links, including partial results. A private
inherited result file and an independent reaper thread let cleanup survive TUI and runtime shutdown.
Failures release pending suppression and restore remaining history; successful suppression lasts until
a fresh client reopens the directory. Workspace files and shared servers
remain independently owned. Service snapshots reconcile
completed edits across in-flight reads. Retained acceptance metadata reflects edits, and deletion
markers prevent stale observations from restoring removed conversation links; event and report
history remain independently owned.
Unknown observations stay explicit, and disabling the integration discards its cache and cancels
observation and navigation without changing externally owned servers or panes. Accepted detached
folder cleanup continues through its result. Companion installation is an
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
Event snapshots include namespace-scoped evaluation/dispatch issue counts for visible records.
The handover feed retains affected events even without an acceptance. Exact diagnostics reads through
`AppService` serve both the TUI and management MCP from persisted evaluations and pinned rule snapshots;
they perform no evaluation, dispatch or external commands. Results group the latest 50 attempts with
explicit truncation and include lineage-verified retained startup log tails. Reused instance names
cannot supply another acceptance's logs. The TUI loads diagnostics on its background service worker;
the user-opened bottom dialog keeps a searchable snapshot beside raw event JSON.

Confirmed event deletion removes namespace-scoped events, processing history, pending rule work,
and associated feedback atomically while holding leases only for the selected events. Dispatch, retry,
and recreation admission hold the same event lease and reread retained acceptance identity before
acting. Unreported provisioning or launching and active conclusion cleanup block deletion of affected history.
Instances, sessions, provider credentials, rule definitions, and namespace dispatch admission history
are preserved. Bulk deletion is atomic when any selected event is busy.
Ignored-event deletion selects all retained namespace events with zero acceptances and removes their
pending work in one database transaction, including pending events; it requires no worker or event lease.
Acceptance deletion removes one acceptance, retained workspace linkage, and assignment feedback while
preserving its event, sibling acceptances, completed evaluations, instances, sessions, and admission
history. Event and acceptance deletion preserve records owned by incomplete provider deletion.

Processing attempts are pending or accepted. Acceptance means a rule matched and durably triggered
an independent action. Explicit replay of any retained event uses current enabled rules with a separate,
idempotent attempt identity. Retained processing history preserves identities and timestamps through
atomic schema migration. Feedback is `received`, `replayed`, or `assigned`; retained `acknowledged`
notifications remain readable. Providers
poll and acknowledge notifications independently of event acceptance. Durable records survive
sidecar/provider disconnects; external provider side effects require idempotency or reconciliation.
Agent assignment, session launch, prompt delivery, and task completion remain separate facts.

`store/rules` defines versioned rule snapshots, bounded boolean Rhai predicates, optional independently
budgeted string hooks for instance names and descriptions, in-memory
Handlebars prompt rendering, and independent per-rule/per-attempt acceptances. Scripts expose no
host I/O. Prompt templates use strict field interpolation and literal event text without HTML
escaping. Rendering bounds output to 64 KiB, nesting to 32 levels, and template evaluations to 50,000;
budgeted block execution also bounds empty loops and inline-partial recursion.
`environments/rules` owns shared definitions in `templates/rules/<name>/rule.json` and namespace-local
activation, saved session metadata, optimistic revisions, cached definitions, pinned evaluations, acceptances,
dispatch admission history, and assignment feedback in SQLite. A shared catalog lock precedes write
transactions for rule saves, discovery, receipt, and replay. New files are inactive; external definition
edits invalidate local activation. File removal hides the rule and disables future matches, retaining
its revision identity and pinned history. Invalid files fail closed. Receipt/replay transactions reconcile
the catalog before selecting rules and
pin every enabled rule revision; completed evaluations survive redelivery and recovery. Match errors
and false results are separate outcomes. An admitted match atomically creates its acceptance and assigned
instance identity and resolved description before external actions, regardless of sibling outcomes.
Hook failures fall back independently. A home-wide, case-normalized name ledger commits with the
acceptance and survives instance and history deletion. Initialization backfills retained acceptance
and runtime names once in the schema transaction; deleted pre-upgrade identities are unrecoverable.
Legacy imports reserve names in their runtime-import transaction, including later namespace imports.
Allocation excludes runtime records from every namespace and all case-insensitive workspace entries,
with at most 1024 suffix candidates. Exhaustion fails the evaluation without an orphan acceptance.

Per-rule acceptance throttling defaults off (`throttle_seconds=0`), regardless of `trigger_at_end`.
SQLite serializes admission by namespace and rule identity in the evaluation transaction.
With a positive duration, the first matching live evaluation opens
a fixed window; later matches retain throttled outcomes and warnings without creating acceptances.
With `trigger_at_end=false`, admission creates the acceptance immediately. With it enabled, the first
match remains a pinned deferred evaluation until its evaluation-time deadline; a worker cycle creates
its acceptance at or after expiry. Future deadlines do not occupy the evaluation batch.
Replay bypasses this gate without reading or changing its window. History deletion preserves windows;
deleting a deferred evaluation cancels its action.
Throttle state survives process restarts and remains independent of dispatch admission.

`service/rules` owns authorization, previews, recovery, and dispatch. Previews use a read transaction
and the prospective acceptance ID; resolved names are advisory and reserve nothing. Fresh dispatch
rechecks name availability under the case-normalized instance lock. Retries verify the originating
operation and template before using retained resources; reservations confer no resource ownership.
Activation verifies that a live Zellij session exists. Saved session names are legacy metadata.
Dispatch resolves the newest live session by creation time before preparation and before its
durable launch marker, including on confirmed retries. Full, unformatted Zellij listings exclude exited,
resurrectable sessions; short listings hide that status. Missing live destinations produce a retained
pre-launch error. Pinned rule definitions remain immutable; the launch receipt records the actual pane.
A private namespace file lease serializes background cycles across TUI and owned event-sidecar
processes. Admission permits four
active actions and ten starts per minute, counting retries. Each action provisions a fresh instance
through the existing detached startup contract. Successful preparation and requested service readiness
gate model/prompt launch; each pinned rule revision owns its default-on service-start and pane-focus
flags. Background launches use Zellij's no-focus creation and skip navigation, preserving all clients'
focus while recording the exact created pane. Manual navigation focuses its requested destination.
The launch marker is durable before contacting Zellij; an interrupted or uncertain launch is surfaced
without automatic resend. Observed exact workspace/pane linkage records the session and `launched`
outcome, which does not certify prompt delivery or task completion. Confirmed pre-launch retries keep
the assigned instance; replay intentionally creates new actions. Assignment and its originating-provider
notification commit together. The sidecar polls automation independently of open UI/MCP clients.

Default acceptance names include the event sequence and durable acceptance ID within the instance-name
length limit. Custom names receive numeric collision suffixes. Names and resolved descriptions remain
fixed through retries and recreation; legacy descriptions use the pinned rule name and event summary.
SQLite retains each acceptance's workspace and observed conversation metadata, excluding live
pane attachment. Headless dispatch captures metadata when it verifies workspace/pane linkage. Purge refreshes
metadata under the instance lock, preserves saved endpoints when observation is unavailable, and verifies
the startup lineage before associating conversations. Event/provider deletion removes dependent
history. Recreation uses the recorded identity with current template files, preserves OpenCode history,
and requires confirmation that workspace files and runtime data will be freshly provisioned. The original
dispatch record remains historical; recreation sends no rule prompt. Reopening a retained conversation
requires its server-side history and server to remain available.

`app/rules` projects rule lists, acceptance histories, and editable bottom dialogs through tuicore.
The Events tree nests acceptances and their conversations beneath each event; rule histories filter by rule identity. Exact history navigation
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

Schema-version-2 provider manifests optionally declare named streams with individual profiles and
cooperative stream-control capability. Schema-version-1 packages and saved launches normalize their
provider profile into each declared stream when read. Event envelopes retain their own concrete profile;
ingestion validates their payload independently of the manifest. Undeclared streams project a profile
only when their retained events agree. Namespace/source/
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
The Streams DataView projects provider parents and stream children with stable identities, independent
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
protocol tests. Streams is the final tab, with lifecycle approval, bounded logs, and exact source
links to the second Events tab.

Provider deletion is an explicitly confirmed MCP service operation. An exclusive package lock excludes
cross-namespace lifecycle work; namespace/provider and source locks exclude collection changes and
identity reuse. The rule-worker lease gates dispatch admission. Durable deletion markers block queued
dispatch and collection restarts across failures and process restarts. Positive Docker labels authorize
collector/checkpoint removal; canonical package/runtime/credential paths bound filesystem cleanup.
Instance purge rechecks the originating startup lineage under the instance lock before closing clients,
deleting workspace OpenCode history and removing resources. External failures retain launch/event
identity for retry. Successful cleanup atomically removes provider credentials, streams, events and
dependent history, launch records and the deletion marker. Shared rules, sidecars, namespace admission
history and Docker caches remain independently owned.

## Growth rules

Keep Tandem as one package until a real boundary appears: another client needs the domain engine, build pressure warrants separation, or deployment/security requires an independent process. Split modules by capability before splitting crates. New infrastructure integrations sit behind `AppService`; new interfaces remain adapters over it.
