# Plan: Stream providers and event-driven work

Status: developer ingestion, provider lifecycle, rule previews, per-rule acceptance history, and guarded instance/session dispatch are implemented. Catalog migration, production-source validation, prompt-delivery acknowledgment, and future handlers remain follow-ups. Checkboxes distinguish implementation evidence from live model execution.

## Goal

Let an agent configure a provider for any external source, regardless of language or whether the source streams or requires polling. Tandem receives normalized events, makes them inspectable in the TUI, and lets users define conditions that turn selected events into agent work in Tandem workspaces.

Initial examples: Slack messages, Jira ticket changes, and observations from a polled system.

### Confirmed first-version scope

1. Display arriving events using common normalized shapes and source-specific metadata.
2. Evaluate every enabled rule's Rhai predicate for each newly received event; every match triggers its own action.
3. Each rule specifies a Tandem instance template as its handler, a session model, and an initial prompt template.
4. Each matched action creates a fresh instance from that template and starts agent work with the resolved prompt.
5. Support four profiles: message, ticket, system event, and generic.
6. Every newly received provider event is eligible for enabled rules, including historical imports and batches. Providers decide what to emit and when; stable event IDs identify redeliveries.
7. Persist evaluation status, acceptance, and action history so delivery retries skip evaluated work and users can explicitly replay retained events. Accepted means a rule matched and triggered its action; startup, prompt delivery, and task completion have separate outcomes.
   Record acceptance per rule and processing attempt. One event can be accepted by several rules, each with its own instance/session and dispatch outcome.
8. Send lifecycle feedback to the originating provider, starting with an assignment notification so provider code can respond in its source system.
9. Keep instance and provider templates in one disk-backed, Git-shareable catalog rooted at `templates/`, with separate `instances/` and `providers/` directories.

Existing-instance routing and ticket/thread-based instance reuse are outside the first-version scope. Enabled revisions authorize automatic execution; dispatch waits for successful instance readiness. Richer profile semantics and verified prompt-delivery acknowledgment remain open. Acceptance is automatic.

## Conceptual model

### Implemented workflow

The second main tab is Events, with distinct DataView rows for all four typed profiles, source
metadata/context inspection, and durable provider-scoped event IDs. Ingestion atomically creates
a pending processing attempt and receipt feedback. Explicit replay of any retained event creates a
separate pending attempt and preserves history. The stored pending/accepted model and schema migration
preserve retained processing records. Legacy acceptances retain their history without fabricated rule matches.
New attempts pin enabled rule revisions; all matching predicates create independent acceptance records.

A separate authenticated HTTP sidecar supports batch ingestion and provider polling/acknowledgment
of `received`, `replayed`, and `assigned` notifications; retained `acknowledged` notifications remain
readable. Tandem owns a detached sidecar per home/namespace and ensures it on provider startup or
rule authorization. It runs guarded automation independently of open TUI/MCP clients.

Developer fixtures provide four Docker packages with durable checkpoints and private token mounts.
Providers is the third tab: approved Start, Stop, Pause, Resume, bounded Logs, configuration inspection,
and source links to Events. `AppService` owns image builds, credentials, sidecar readiness, launch
snapshots, and verified Docker lifecycle. Start and Resume repair sidecar interruption while preserving
its address. Collector and sidecar lifetimes are independent of open TUI clients.
Fresh fixtures share one Git catalog with `instances/` and `providers/`; provider packages can be
appended to existing flat fixture catalogs without moving their files. Full catalog migration remains
an explicit pending phase. Current projections show the latest 200 events and 50 attempts per event;
automatic history retention/pruning remains open.

### Domain concepts

1. **Provider definition:** a reusable recipe containing an image or Docker build context, configuration requirements, and its event contract. Source-specific code belongs here.
2. **Provider installation:** a configured, independently managed runtime of a definition, with credentials, persistent source checkpoints, and lifecycle status. The implemented model has one installation per package and namespace; multiple installations of one definition are a later extension.
3. **Event:** an immutable observation emitted by an installation and durably received by Tandem.
4. **Stream:** a named source feed from an installation. A provider can expose several streams; a combined TUI view can filter across them.
5. **Rule:** an enabled/disabled Rhai predicate over received events plus an instance handler template, session model, and initial prompt template.
6. **Dispatch:** a durable record of a rule match and the resulting proposed or executed action, linked to the event, workspace, instance, and agent session where available.
7. **Processing attempt:** a durable pass over an event, containing rule evaluation and dispatch outcomes. Initial processing and explicit replay have separate attempt identities while retaining the same source event identity.
8. **Provider notification:** a durable, correlated lifecycle message sent to the originating installation. Notification delivery has its own status, independent of event processing and agent execution.

Providers ingest and normalize. Tandem owns event storage, inspection, rule evaluation, and dispatch. Providers do not decide which Tandem instances to create or which prompts to execute.

## Common events versus stream types

**Confirmed direction:** one versioned event envelope with a declared profile for its normalized payload: `message`, `ticket`, `system_event`, or `generic`.

Providers normalize their source into the chosen profile and place custom fields in metadata. Implemented v1 payloads require message author/channel/text, ticket key/title/status, system resource/signal/severity/description, or an object-valued generic payload. Message thread and ticket assignee are optional. Rules inspect normalized fields and metadata; the richer field groups below remain proposals.

Profiles share ingestion, persistence, filtering, and dispatch semantics. Each predicate receives the same envelope with a declared `message`, `ticket`, `system_event`, or `generic` profile, normalized `data`, and an arbitrary JSON-object `metadata` field. Start with one chronological feed and profile-aware event details; dedicated message or board views can follow separately.

A board is a projection of entity state, not a different transport. Ticket events need a stable entity key and an explicit way to interpret snapshots, changes, and deletions before a board can be reliable.

Validate the initial profiles against representative source events before fixing their field names, required fields, and versioning rules.

### Shared context

People, attachments, source links, and related work are shared capabilities rather than profile-specific extensions. Normalize these into common structures so the TUI and prompt builder can use them across all four profiles. Custom metadata remains available for source-specific details.

Candidate shared context structures:

1. **People:** source-scoped identity, display name, and relationship to this event or entity, such as author, assignee, mention, or participant. A shared display name does not prove two source identities are the same person.
2. **Attachments:** identity, name, media type, optional byte size, and a source reference. References are not downloaded file contents. Protected attachments need an authorized resolution mechanism; arbitrary provider URLs must not cause automatic fetching or credential forwarding.
3. **Relations:** source-scoped keys and links with a relationship type, such as parent, reply-to, blocks, relates-to, or associated change. These can connect a ticket to a conversation, repository change, or deployment without requiring all related objects to be locally present.
4. **Context:** bounded supporting observations supplied by the provider, with provenance and completeness/truncation status. A related object included as context is not automatically a separately actionable event.

### Profile extensions to refine

| Profile | Distinct structure | Useful presentation and rule behavior |
| --- | --- | --- |
| `message` | Conversation/channel key, thread key, message key, author, text, reply relation, and edit/delete semantics. | Group messages into threads, show conversational context and participants, and match messages or mentions. Applies to email, Slack, and Teams. |
| `ticket` | Stable work-item key, title, description, status, assignees, and explicit snapshot/change/delete semantics. | Show a work-item detail view with related work, attachments, and status changes; match transitions or assignment. A board requires a later state projection. |
| `system_event` | Affected resource/component, signal kind, severity, observed condition, and optional incident/correlation key. | Filter by severity or resource and group related operational signals. Examples include CI failure, deployment completion, health-check failure, alert firing/resolution, or a relevant log record. |
| `generic` | Shared envelope and context with an otherwise unconstrained JSON payload. | Show a summary, links, context, and inspectable data; permit conditions on payload and metadata without claiming domain-specific behavior. |

The profiles are useful because they establish interpretable relationships and behaviors, not because they permit extra JSON fields. Optional shared context should not force providers to manufacture information their source does not supply.

Distinguish event identity from entity identity: several events can describe changes to one ticket, messages in one thread, or observations of one resource. The entity key groups them; a distinct event ID identifies each independently actionable observation. Conversation context needs its own ordering and completeness semantics; feed arrival order alone cannot reconstruct a thread.

For message rules, define whether a match receives only the triggering message, a provider-supplied thread snapshot, or a referenced thread resolved later. The first version should expose available bounded context without claiming it is a complete conversation. Provider enrichment must not silently create extra dispatches.

### Implemented v1 envelope

| Field | Purpose |
| --- | --- |
| `schema_version` | Version of the Tandem ingestion contract. |
| `event_id` | Provider-supplied identity, stable across delivery retries. |
| `stream` | Feed key within the installation. |
| `profile` | Declared normalized payload shape, independent of the source-specific event type. |
| `type` | Namespaced event type, such as `slack.message.created` or `jira.issue.updated`. |
| `occurred_at` | Source timestamp, when known; absence remains explicit. |
| `subject` | Optional stable entity key, such as a ticket or message identifier. |
| `summary` | Short plain-text description suitable for a generic feed. |
| `body` | Optional human-readable content for inspection and prompt context. |
| `url` | Optional link to the source object. |
| `people`, `attachments`, `relations`, `context` | Optional shared context. People contain id/name/role; attachments name/url/optional media_type; relations kind/subject/optional url; context is provider-supplied JSON. |
| `data` | Normalized payload validated against the declared profile. |
| `metadata` | Optional source-specific JSON object within the 64 KiB event limit. |

Tandem assigns the authenticated installation identity, receipt timestamp, and local ingestion sequence. Provider input cannot impersonate another installation. Deduplication uses namespace, installation identity, and `event_id`; providers keep IDs stable across retries and distinct across their streams. Reusing an ID for different normalized content rejects the whole batch with HTTP 409.

Every newly received event is eligible for enabled rules regardless of its source timestamp or whether it arrived in a historical import or batch. Ten distinct events in one submission must all be accounted for; Tandem may queue their actions under concurrency limits, but must not silently sample, merge, or discard them. Nonmatching events remain visible without creating instances or becoming accepted.

Providers own the decision to emit backfill, repeated observations, or actual changes. To redeliver the same observation, reuse its ID; to report a separately actionable change, supply a new ID even when the subject is unchanged. Checkpoint advancement must follow durable receipt, and identifiers must survive provider restart. Rule enablement is prospective. Confirmed replay of any retained event pins current enabled revisions; bulk scanning remains outside this slice.

## Provider contract and lifecycle

### Shared template repository

The template root is one Git repository. Instance recipes live under `templates/instances/<name>/`; provider recipes live under `templates/providers/<name>/`. Each recipe has its own directory for manifests, Dockerfiles, source code, dependency files, scripts, and static configuration.

```text
templates/                       # shared repository root
  .git/
  instances/
    website/
      tandem.json
      compose.yaml
      scripts/
  providers/
    slack/
      Dockerfile
      provider.json              # validated provider manifest
      src/
      config/
```

These paths are relative to the configured Tandem home. `provider.json` validates schema_version 2, a stable unique name, description, protocol `tandem-events-v1`, optional `{name, profile}` stream declarations, and optional feedback names. Each stream selects one of the four profiles independently. Each package owns its Docker build context. MCP exposes package metadata and runtime state; full supporting-file tree/content inspection remains pending.

Discovery scans the two catalogs independently. Template identity includes its kind, allowing an instance template and provider template to share a name. A rule's instance-template reference resolves only against `instances/`; a provider installation's definition reference resolves only against `providers/`. The existing instance-template tools retain their domain; provider authoring receives separate service/MCP operations.

The shared repository contains reusable recipes and nonsecret defaults. Credentials, configured installation state, checkpoints, event records, processing history, notification queues, generated runtime files, and writable volumes live in Tandem-owned local storage outside the template checkout. Provider execution must preserve the entire shared template repository, not only its own directory.

Future handlers are outside the first-version scope. The proposed storage boundary is Git-controlled handler definitions and supporting files under `templates/handlers/<name>/`, alongside the instance and provider catalogs. Handler behavior, package contracts, and integration with rules remain undefined.

SQLite owns handler run records, status transitions, attempts, resolved inputs/prompts, links to events and instances, and outcomes. A run retains the exact applied nonsecret definition snapshot and content digest so an edited or deleted working-tree definition cannot change queued work or obscure its provenance; a Git commit alone is insufficient when definitions contain uncommitted edits. Replay must explicitly select the historical definition or the current one.

Large logs and binary artifacts may live in private runtime storage, with references and lifecycle metadata in SQL. Credentials remain private references rather than copied values in definitions, run snapshots, or Git. The durable database and runtime artifacts are local execution state, independent of the shared template repository.

Catalog relocation needs an explicit, validated migration. Preserve the repository root, Git metadata, uncommitted files, template names, and existing instance identities; refuse ambiguous entries and destination collisions. Define recoverable migration steps and coordinate all readers/writers before moving files. Verify saved launch snapshots and path references remain usable for existing-instance restart and cleanup. Update template discovery, path reporting, observers, ownership guards, fixture generators, documentation, and agent placement guidance together.

An agent should be able to author the provider files, validate the definition through MCP, configure an installation, and request an approved build/start. Image execution and secret access require explicit user approval; permission to write files alone does not authorize either.

Provider packages should describe configuration fields, secret references, emitted stream keys, and supported event types. Pin the applied configuration/build identity so editing a definition does not silently change a running installation.

Installations need explicit start, stop, restart, status, and bounded diagnostic access. Distinguish a running container, a healthy collector, and a stream that has received recent events; quiet sources are not necessarily broken.

Persist source checkpoints in installation-owned storage. Stop preserves state. Deletion must distinguish runtime cleanup from deletion of retained events and credentials.

### Implemented transport

Providers submit authenticated HTTP batches of 1–100 events, with a 64 KiB event limit and 1 MiB request limit. Ingestion atomically stores each distinct event, its pending attempt, and receipt feedback before returning acknowledgments. Delivery is at least once with durable deduplication; sample providers persist a pending batch until acknowledgment.

Providers poll and acknowledge their own feedback queue. Tandem prepares private token mounts and a leased native sidecar, supplies its verified loopback address, and preserves that address on recovery. Managed Docker collectors use Linux host networking. Source-rate limits, production backpressure, storage retention, and broader deployment/networking policies remain open; exactly-once external execution is not guaranteed.

### Lifecycle feedback to providers

Providers consume `received`, `replayed`, and `assigned` notifications as well as producing source events; retained `acknowledged` notifications remain readable. Assignment means a specific action has been durably bound to a Tandem instance and a startup operation admitted. It does not imply instance readiness, prompt delivery, or task completion.

A Slack provider could add a reaction or thread reply linking the work; a ticket provider could add a comment or update a field. Source-specific responses belong in provider code and use explicitly configured source permissions. Tandem defines the notification contract rather than embedding Slack or Jira behavior.

Notification fields include stable `notification_id`, original provider `event_id`, local sequence, processing-attempt ID, lifecycle kind, and creation timestamp. Assigned feedback adds dispatch ID, rule name/revision, and instance. Only the originating installation can read it. Multiple matched actions or explicit replay can produce separate assignments for the same source event; delivery retries keep the same notification ID.

Assignment and its notification outbox entry commit together. Authenticated provider polling delivers at least once until acknowledgment. Polling cadence/backoff and source-operation reconciliation belong to providers; delivery failures do not undo assignment or recreate instances. Provider handlers need durable notification-ID deduplication; acknowledgment cannot guarantee exactly-once source-system updates. TUI feedback-delivery diagnostics remain a follow-up.

Feedback uses authenticated provider polling and idempotent acknowledgment. Notifications survive provider/Tandem restart and remain queued until acknowledgment. Production feedback retention and callback delivery, if needed, are future decisions.

Treat lifecycle notifications as a separate protocol direction, not source events fed back into the rule matcher. Provider-authored source updates can still return through the source stream; define provenance/correlation and rule exclusions to prevent those updates from creating automation loops. Provider code may ignore notifications it does not use; support should be declared as a capability.

Reserve later lifecycle notifications for independently verified transitions. `accepted` records a rule trigger. Assignment, provider acknowledgment, acceptance, and agent inactivity cannot substitute for confirmed task success.

## Inspection and automation

### Implemented TUI and automation

Events is second and Providers third. Events renders distinct profile rows, details, metadata/context, pending/accepted attempts, and replay. Providers exposes approved lifecycle controls, fresh Docker state, errors, details, and bounded logs. Source links select the matching provider or apply an exact event-source filter. Rendering sanitizes external text and arrival updates preserve selection while the event remains in the bounded feed.

The two-row Events layout displays `Accepted · 3 rules` for an event accepted by three distinct rules in its latest processing attempt. Its bottom dialog includes an **Acceptances** tab listing each matched rule, acceptance timestamp, processing attempt, instance/session links, and dispatch status. Replay history retains its separate acceptance records.

The Rules main tab lists rules in a DataView with two rendered rows per rule. `.` opens the shared action menu with hotkey labels. `a` activates or deactivates the selected rule; Space also toggles it. A top-right button above the DataView uses `A`: Activate all targets inactive rules, while Deactivate all appears when every rule is active. Bulk confirmation pins eligible saved revisions and authorizes each activation; failed saves preserve successful sibling outcomes. Enter opens the standard bottom-docked dialog with these tabs:

1. **Accepted events:** only retained events that triggered this particular rule, linked to their processing attempt and action outcome. Enter switches to the main Events tab and focuses the exact event; history navigation must load retained events outside the latest-200 feed and clear conflicting filters. Several rules can list the same event.
2. **Script:** the rule's Rhai predicate source with Rust syntax highlighting.
3. **Settings:** tuicore form controls for the rule's description, enabled state, session model, initial prompt template, and instance handler template.

Valid field edits persist automatically and keep the editor open. Invalid drafts retain the last
saved values and show an error. Editing pauses an enabled rule before draft validation; enabling
its saved revision requires authorization. Queued writes retain the latest edits across dialog
closure. The outer dialog owns close chrome. Predicates inspect custom provider fields through
the incoming event's metadata.

MCP creates, reads, validates, updates, and enables/disables rules. Match and resolved-prompt previews perform no instance creation or agent contact.

### Rule execution

A rule contains a Rhai function `matches(event)` returning a boolean. It may inspect the normalized event envelope, profile-specific data, shared context, and metadata. Predicates are pure: expose no filesystem, network, process, or domain-mutation capabilities, and enforce operation, recursion, and allocation limits. Compilation errors, runtime errors, and nonboolean results are visible evaluation failures, not false matches; one failure must not prevent evaluation of the other rules.

Each receipt/replay transaction pins enabled revisions in namespace-local SQLite evaluation rows. Workers evaluate every pinned predicate and persist matched, no-match, or failed outcomes independently. Every true result atomically creates an acceptance/action with its unique assigned instance identity and marks that attempt accepted. Acceptance does not short-circuit siblings or depend on dispatch outcomes. Disabled predicates do not run. Optimistic revisions reject stale saves; enabled saves require authorization for automatic future actions.

Acceptance identity includes the rule and processing attempt, with the applied revision pinned in the record. Retry resumes that acceptance's assigned instance/session; replay creates acceptances under a new attempt. One rule's failed launch must neither hide another acceptance nor repeat another rule's successful action.

Each matched action creates a fresh instance. Persist a unique action identity and its assigned instance name before creation so delivery retries and worker restarts cannot create extra instances for the same action.

Dispatch waits for successful detached instance startup, including repository and service readiness, before requesting a fresh conversation. Model and optional variant are passed with the resolved prompt as literal arguments. The launch marker commits before contacting Zellij; observed exact workspace/pane linkage establishes the session and launched state. Failed preparation or uncertain launch remains visible while the event stays accepted. Session presence does not verify prompt delivery or task completion.

Authoring validates scripts, prompt bounds, model syntax, and the referenced template. Provisioning revalidates the current template; OpenCode resolves model availability and credentials. Failures remain scoped to their acceptance. A model-capability preflight and verified prompt acknowledgment remain follow-ups.

Match and prompt previews perform pure evaluation only. Authorizing an enabled revision covers subsequent automatic template execution, host credentials, model prompts/costs, and creation-history cleanup when enabled. Stream text is untrusted task input, not authorization or higher-priority instructions. Prompts use single-pass event-field interpolation with a 64 KiB resolved limit and never expose provider credentials.

### Durable processing and replay

Persist a rule match before external side effects. Deduplicate delivery retries separately from intentional replay. Record the rule revision and resolved action so later edits do not alter queued work.

Receipt and acceptance are separate facts. Atomically store a newly received event and its initial pending processing attempt before acknowledging receipt. A duplicate provider delivery returns the existing receipt and leaves processing status intact: it neither resets accepted work nor removes pending work.

Attempt states are `pending` and `accepted`; completed evaluation outcomes are independently retained, including `no_match`. Dispatch states are queued, provisioning, launching, launched, failed, and uncertain. A private namespace file lease serializes workers; recovery observes the persisted startup identity and pane before advancing, preserving successful sibling actions.

Workers select attempts with unfinished evaluation and skip completed evaluations. Record a completed evaluation with no matching rules as `no_match`, without accepting the event. For matched events, retain each rule's action separately; partial success must not cause successful actions to run again when a failed action is retried. An accepted event can still have unfinished evaluation or failed actions, so acceptance alone must not suppress the remaining predicates or their recovery.

Confirmed replay creates a new attempt against current enabled revisions for the same immutable event. It preserves previous acceptances, statuses, and instance/session links without changing provider identity or deduplication. Preview first. Repeated submission of one replay request resolves to the same attempt; historical-snapshot replay is outside this slice.

Retry and replay are different operations. Retry reconciles or resumes an existing failed action with its assigned instance identity; replay intentionally allows fresh actions and instances. Uncertain external outcomes need reconciliation or explicit user resolution before either operation can safely repeat the affected side effect.

Replay accepts any retained event and requires explicit approval. Each replay can trigger the same rule again with a fresh instance. Restricted retry accepts only pre-launch failures with a resolved prompt and preserves the assigned instance identity. An uncertain launch requires inspection and deliberate user resolution; automation never resends it.

Dispatch admits four active actions and ten starts per namespace per minute, counting retries. Startup ownership mismatches fail closed; failed preparations preserve work for explicit retry. Provider-side source exclusions must prevent feedback loops; rate limits bound bursts but do not prove loop freedom.

Redelivery of a received event ID must not create a new processing attempt. Local replay should default to inspection or match preview; deliberate execution needs explicit authorization and a new attempt identity. Retention must preserve enough ingestion and dispatch identity to prevent old redeliveries from becoming new work. Replay requires a retained payload; compact deduplication receipts can still prevent duplicate processing after payload expiration.

## Fit with Tandem

All provider lifecycle, ingestion, storage, rules, and dispatch operations enter through `AppService`. TUI, CLI, and MCP remain adapters. Provider runtime and ingestion I/O belong in environment adapters; typed events and rules belong in domain stores.

SQLite stores durable events, attempts, provider credentials, rule revisions/snapshots, independent acceptances, dispatch admission history, startup outcomes, and notification queues in the private application database. Production volume and retention need validation.

Tandem initializes an application service per process. Provider lifecycle uses namespace/package locks and positive Docker ownership; the detached sidecar holds an identity-verified namespace lease. Rule workers hold a separate namespace file lease through each evaluation/dispatch cycle and reconcile persisted startup/session identities after process interruption.

Provider containers should have ownership separate from instance containers. Stopping or deleting a target instance must not implicitly stop unrelated collection. Reuse established ownership and lifecycle safeguards without assuming an application instance is the right provider abstraction.

Provider networking must not accidentally expose the existing loopback MCP endpoint. Keep credentials out of shared definitions, event records, TUI output, and logs. Trusted Docker builds are not a sandbox; privileges and host mounts need explicit boundaries.

Update runtime agent guidance when the approved provider-authoring and automation workflow is defined. Describe guarantees separately from provider conventions.

## Delivery checklist

Checked tasks have implementation and automated evidence. Unchecked tasks remain work; catalog migration, real-source validation, and live model execution are independent follow-ups.

### 0. Establish the shared template catalogs — partial

- [ ] Define an approved, recoverable catalog migration that preserves the template repository and existing instance records, with collision and unsafe-path checks.
- [x] Support independent `instances/` and `providers/` catalogs; report their roots through MCP, preserve flat catalogs, and reject mixed instance layouts.
- [x] Generate one Git-controlled template catalog containing instance recipes and four independently buildable provider packages.
- [x] Update catalog path consumers, observation, whole-repository mount protection, fixture paths, documentation, and agent guidance; retain existing lifecycle compatibility.
- [ ] Expose provider supporting-file trees/content through MCP inspection.

- [ ] **Checkpoint:** validate an approved relocation of an existing flat catalog, preserve Git/workspaces/saved launches, and inspect complete provider packages through MCP.

### 1. Prove a common contract — v1 implemented, real-source validation pending

- [ ] Collect representative Slack, Jira, and polled-system observations, including duplicates, backfill, edits, and deletion where applicable.
- [x] Implement a versioned envelope and typed v1 message, ticket, system-event, and generic payloads, with shared context and custom metadata.
- [x] Implement stable provider-scoped event IDs, atomic HTTP batch ingestion, conflict rejection, and persistent provider checkpoint/retry behavior.
- [x] Implement authenticated polling/acknowledgment for durable `received`, `replayed`, and `acknowledged` feedback.
- [x] Implement sidecar worker leases/identity probes, package-scoped Docker ownership, private atomic credential files, and read-only token mounts.
- [x] Enforce 64 KiB events, 1 MiB requests, 100-event batches, and bounded feed/history projections.
- [ ] Validate richer profile semantics against real source observations, including edits, deletions, ordering, and context completeness.
- [x] Implement `assigned` feedback atomically with instance/startup admission and preserve notification identities across retry.
- [ ] Specify production rate/backpressure and retention policies.

- [ ] **Checkpoint:** real Slack, Jira, and polled-system examples map into v1 or an approved extension without losing inspection or routing information.

### 2. Run providers and inspect their events — complete

- [x] Validate provider packages and manage approved Start, Stop, Pause, Resume, and bounded Logs through `AppService`, TUI, CLI, and MCP.
- [x] Deliver four Docker sample providers with normalized payloads, shared context/metadata, durable delivery, and feedback consumption.
- [x] Provision credentials, start/verify the owned sidecar, build images, save launch snapshots, and verify collector runtime state without manual shell setup.
- [x] Preserve checkpoint volumes and events across Stop/Start; freeze and resume collection without rebuilding.
- [x] Persist pending attempts atomically with events and replay any retained event with idempotent request identities and preserved history; retain pending/accepted identities and timestamps across schema upgrades.
- [x] Render Events second and Providers third with profile-specific rows, runtime/error state, details, logs, selection preservation, and exact source links/filters.
- [x] Verify source and sidecar restarts preserve received events/checkpoints; recover the sidecar at its recorded address without duplicate ownership.
- [x] Stop an owned installation after its package directory is removed using retained identity and fresh Docker evidence.
- [x] Document the implemented provider-authoring/lifecycle and event protocol, including approval, credentials, ownership, and recovery boundaries.

- [x] **Automated checkpoint:** four profiles deliver 40 distinct Docker events with acknowledged feedback; redelivery/restart retain identities; adapter tests preserve acceptance/replay history; lifecycle tests prove Pause/Resume, Stop/Start, sidecar recovery, Logs, and Stop after package deletion.

### 3. Preview rules against normalized events — implemented

- [x] Add bounded, pure Rhai predicates over all four normalized profiles and arbitrary metadata, with each rule selecting a handler template, session model, and initial prompt.
- [x] Add the two-row Rules DataView, on/off controls, and bottom-docked Accepted events / Script / Settings dialog using tuicore components.
- [x] Link each rule's accepted-event history to exact event focus in the Events tab, including retained events outside the bounded feed.
- [x] Render the latest-attempt accepted-rule count in Events and expose all per-rule/per-attempt records in the event dialog's Acceptances tab with timestamps, instance/session links, and dispatch status.
- [x] Expose rule authoring, validation, enablement, and inspection through MCP via `AppService`.
- [x] Preview matches and rendered prompts without creating instances or contacting agents.
- [x] Define revision authorization, local validation/storage, prospective enablement, and current-revision replay; verify all predicates and independent matches.
- [ ] Preflight configured OpenCode model capabilities and credentials before automatic admission.

- [ ] **Checkpoint:** representative Slack and Jira events match the intended rules and display the selected template and resolved initial prompt.

### 4. Create a fresh instance and start agent work — implemented, live prompt proof pending

- [x] Persist versioned rule matches, per-rule acceptance history, and action identities atomically before creating fresh instances through `AppService`.
- [x] Launch the configured model/variant and literal prompt after successful instance readiness, under enabled-revision authorization.
- [x] Add bounded concurrency/start-rate admission and visible provisioning, launch, and uncertain outcomes.
- [x] Link events to instances and sessions; verify duplicate delivery and interrupted-launch recovery preserve successful siblings.
- [x] Persist matched/no-match/failed evaluations and independent action failures; preserve historical acceptance records through current-rule replay.
- [x] Persist `assigned` notifications atomically with admitted instance assignment and deliver through the originating provider's authenticated retry queue.
- [ ] Verify actual model prompt delivery and add an acknowledgment distinct from session/pane linkage.
- [ ] Verify source-specific feedback exclusions and external side-effect reconciliation prevent loops.

- [ ] **Checkpoint:** a matching event creates a fresh instance from its rule's template, starts its initial prompt, and produces correlated assignment feedback to its provider; notification retries preserve the assignment, and explicit replay records a distinct action and notification.

### 5. Prove sustained operation — developer recovery verified, production work pending

- [x] Verify distinct batch events are retained, identical redelivery is deduplicated, conflicting content rolls back the batch, and receipt loss preserves identical provider retry payloads.
- [x] Verify durable notification retry/acknowledgment and sample-provider notification-ID deduplication.
- [ ] Exercise sustained real-source volume, collector health/source freshness, production backpressure, history retention, and arbitrary log/payload secret handling.
- [x] Verify current-rule replay pins a fresh attempt and retains independent historical actions.
- [ ] Exercise real-source historical imports under enabled-rule authorization.
- [ ] Prevent feedback-driven source-system update loops and verify external side-effect reconciliation.
- [x] Document rule authoring, instance mapping, dispatch, and replay; verify MCP contracts and authorization.

- [ ] **Checkpoint:** a chosen real source runs unattended within its limits, and users can explain every automated action from retained event and dispatch records.

### Verification evidence

- `cargo test --all-targets`: 609 passed, 4 ignored at the last full verification, including acceptance-schema migration, history/replay preservation, and event TUI coverage.
- `python3 -m unittest discover -s projects-generators/tests -p 'test_*.py'`: 12 passed.
- `cargo clippy --all-targets -- -D warnings`, `cargo check --release`, formatting, and diff checks passed.
- `python3 projects-generators/tests/events_smoke.py --docker`: four profiles, 40 durable events, feedback acknowledgment, deduplication, and sidecar restart passed.
- `python3 projects-generators/tests/providers_smoke.py`: discovery, automatic setup, four managed collectors, Pause/Resume, Logs, checkpoint-preserving Stop/Start, sidecar recovery, and template-independent Stop passed.

These are implementation-time results; rerun relevant checks after subsequent edits. Interactive TUI behavior is covered by terminal-backend tests, including narrow layouts, confirmation, tab focus, metadata inspection, and exact provider/event links.

## Decisions for iteration

1. **Shared context and profile fields:** what should Tandem normalize for people, attachments, relations, threads, work-item changes, and operational signals?
2. **Context delivery:** provider-supplied snapshots or later authorized resolution of thread context and attachments?
3. **Authorization — implemented:** enabled revisions authorize automatic future actions; editing an enabled revision requires approval.
4. **Prompt gate — implemented:** wait for successful full instance readiness; prompt acknowledgment is a separate follow-up.
5. **Multiple matches — confirmed:** evaluate every enabled predicate; create one independent action and fresh instance per matching rule.
6. **Provider packaging:** Docker image/build only, or Compose recipes for providers needing multiple services?
7. **Dispatch ownership — implemented:** namespace file lease, pinned identities, startup reconciliation, and explicit uncertain outcomes without automatic resend.
8. **Rule enablement — implemented:** prospective matches; scanning retained nonmatching events remains outside this slice.
9. **Concrete example:** which event, condition, template, and initial prompt should prove the first end-to-end workflow?
10. **Acceptance boundary — confirmed:** a rule matched and durably triggered its action; acceptance is automatic, and startup, prompt delivery, and task completion remain separate outcomes.
11. **Replay rules — implemented:** confirmed replay pins current enabled revisions; historical-snapshot replay is a later extension.
12. **Assignment feedback — implemented:** durable startup operation admission plus correlated rule/action/instance feedback through authenticated polling.
