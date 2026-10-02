# Plan: Stream providers and event-driven work

Status: developer ingestion and provider lifecycle are implemented and verified. Catalog migration and production contract validation are partial; rules, instance mapping, dispatch, and handlers are pending. Checkboxes below track delivered work separately from deferred extensions.

## Goal

Let an agent configure a provider for any external source, regardless of language or whether the source streams or requires polling. Tandem receives normalized events, makes them inspectable in the TUI, and lets users define conditions that turn selected events into agent work in Tandem workspaces.

Initial examples: Slack messages, Jira ticket changes, and observations from a polled system.

### Confirmed first-version scope

1. Display arriving events using common normalized shapes and source-specific metadata.
2. Match events against configured rules.
3. Each rule specifies a Tandem template and initial prompt.
4. Each matched action creates a fresh instance from that template and starts agent work with the resolved prompt.
5. Support four profiles: message, ticket, system event, and generic.
6. Every newly accepted provider event is eligible for enabled rules, including historical imports and batches. Providers decide what to emit and when; stable event IDs identify redeliveries.
7. Persist processing status and history so handled events are skipped automatically and users can explicitly replay retained events.
8. Send lifecycle feedback to the originating provider, starting with an assignment notification so provider code can respond in its source system.
9. Keep instance and provider templates in one disk-backed, Git-shareable catalog rooted at `templates/`, with separate `instances/` and `providers/` directories.

Existing-instance routing and ticket/thread-based instance reuse are outside the first-version scope. The v1 profiles are implemented; richer profile semantics, rule authorization, and prompt delivery timing remain open.

## Conceptual model

### Implemented developer slice

The second main tab is Events, with distinct DataView rows for all four typed profiles, source
metadata/context inspection, and durable provider-scoped event IDs. Acceptance atomically creates
a pending processing attempt and receipt feedback. Manual acknowledgment marks an attempt handled;
explicit replay of a handled event creates a separate pending attempt and preserves history.
Acknowledgment in this slice is a user decision, not evidence of completed agent work.

A separate authenticated HTTP sidecar supports batch ingestion and provider polling/acknowledgment
of `received`, `replayed`, and `acknowledged` notifications. Tandem owns a detached sidecar per
home/namespace and starts it as part of provider startup. Agent assignment, handlers, rules, and automatic
instance creation remain later work.

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
3. **Event:** an immutable observation emitted by an installation and durably accepted by Tandem.
4. **Stream:** a named source feed from an installation. A provider can expose several streams; a combined TUI view can filter across them.
5. **Rule:** a condition over accepted events plus an instance template and initial prompt template.
6. **Dispatch:** a durable record of a rule match and the resulting proposed or executed action, linked to the event, workspace, instance, and agent session where available.
7. **Processing attempt:** a durable pass over an event, containing rule evaluation and dispatch outcomes. Initial processing and explicit replay have separate attempt identities while retaining the same source event identity.
8. **Provider notification:** a durable, correlated lifecycle message sent to the originating installation. Notification delivery has its own status, independent of event processing and agent execution.

Providers ingest and normalize. Tandem owns event storage, inspection, rule evaluation, and dispatch. Providers do not decide which Tandem instances to create or which prompts to execute.

## Common events versus stream types

**Confirmed direction:** one versioned event envelope with a declared profile for its normalized payload: `message`, `ticket`, `system_event`, or `generic`.

Providers normalize their source into the chosen profile and place custom fields in metadata. Implemented v1 payloads require message author/channel/text, ticket key/title/status, system resource/signal/severity/description, or an object-valued generic payload. Message thread and ticket assignee are optional. Future rules will inspect normalized fields and metadata; the richer field groups below remain proposals.

Profiles share ingestion, persistence, filtering, and dispatch semantics. Start with one chronological feed and profile-aware event details; dedicated message or board views can follow separately.

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

Every newly accepted event is eligible for enabled rules regardless of its source timestamp or whether it arrived in a historical import or batch. Ten distinct events in one submission must all be accounted for; Tandem may queue their actions under concurrency limits, but must not silently sample, merge, or discard them. Nonmatching events remain visible without creating instances.

Providers own the decision to emit backfill, repeated observations, or actual changes. To redeliver the same observation, reuse its ID; to report a separately actionable change, supply a new ID even when the subject is unchanged. Checkpoint advancement must follow durable acceptance, and identifiers must survive provider restart. Rule enablement is prospective by default as a proposal; scanning already accepted events under a new rule needs an explicit policy.

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

These paths are relative to the configured Tandem home. `provider.json` validates schema_version 1, a stable unique name, one of the four profiles, description, protocol `tandem-events-v1`, and optional feedback names. Each package owns its Docker build context. MCP exposes package metadata and runtime state; full supporting-file tree/content inspection remains pending.

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

Providers submit authenticated HTTP batches of 1–100 events, with a 64 KiB event limit and 1 MiB request limit. Acceptance atomically stores each distinct event, its pending attempt, and receipt feedback before returning acknowledgments. Delivery is at least once with durable deduplication; sample providers persist a pending batch until acknowledgment.

Providers poll and acknowledge their own feedback queue. Tandem prepares private token mounts and a leased native sidecar, supplies its verified loopback address, and preserves that address on recovery. Managed Docker collectors use Linux host networking. Source-rate limits, production backpressure, storage retention, and broader deployment/networking policies remain open; exactly-once external execution is not guaranteed.

### Lifecycle feedback to providers

Providers consume `received`, `replayed`, and `acknowledged` notifications as well as producing source events. The next automation notification is proposed as `assigned`: a specific action has been durably bound to a Tandem instance whose startup has been accepted. Assignment does not imply instance readiness, prompt delivery, or task completion; its transition remains to be implemented with dispatch.

A Slack provider could add a reaction or thread reply linking the work; a ticket provider could add a comment or update a field. Source-specific responses belong in provider code and use explicitly configured source permissions. Tandem defines the notification contract rather than embedding Slack or Jira behavior.

Proposed notification fields: contract version, stable `notification_id`, original provider `event_id`, processing-attempt ID, dispatch ID, rule identity/revision, lifecycle type, instance identity, and transition timestamp. Send only the originating installation's notifications and keep credentials out of notification payloads. Multiple matched actions or explicit replay can produce separate assignments for the same source event; delivery retries keep the same notification ID.

Record the assignment transition and notification outbox entry in the same transaction. Deliver asynchronously with at-least-once semantics, acknowledgment, bounded retry backoff, and visible failed/undelivered status. A notification-delivery failure must not undo the assignment or cause another instance to be created. Provider handlers need durable notification-ID deduplication and source-operation idempotency or reconciliation where available; acknowledgment alone cannot guarantee exactly-once external updates.

Feedback uses authenticated provider polling and idempotent acknowledgment. Notifications survive provider/Tandem restart and remain queued until acknowledgment. Production feedback retention and callback delivery, if needed, are future decisions.

Treat lifecycle notifications as a separate protocol direction, not source events fed back into the rule matcher. Provider-authored source updates can still return through the source stream; define provenance/correlation and rule exclusions to prevent those updates from creating automation loops. Provider code may ignore notifications it does not use; support should be declared as a capability.

Reserve later lifecycle notifications for independently verified transitions. `handled` requires the agreed completion boundary and an explicit outcome; assignment, provider acknowledgment, and agent inactivity cannot substitute for confirmed task success.

## Inspection and automation

### Implemented TUI and future automation

Events is second and Providers third. Events renders distinct profile rows, details, metadata/context, pending/handled attempts, and replay. Providers exposes approved lifecycle controls, fresh Docker state, errors, details, and bounded logs. Source links select the matching provider or apply an exact event-source filter. Rendering sanitizes external text and arrival updates preserve selection while the event remains in the bounded feed.

An automation section for inspecting rules, previewing matches, and following dispatch outcomes remains pending; its placement is open.

### Rule proposal

A rule selects installations/streams, profiles, and event types, tests declarative conditions on normalized fields or metadata, and specifies an instance template plus initial prompt template. Avoid arbitrary executable predicates in the first version.

Each matched action creates a fresh instance. Persist a unique action identity and its assigned instance name before creation so delivery retries and worker restarts cannot create extra instances for the same action.

The initial prompt starts a new agent conversation in the created instance's workspace. Specify when dispatch may proceed: workspace preparation, complete instance readiness, or another explicit gate. Tandem can launch a client before provisioning finishes, so client presence alone cannot prove the selected gate passed. Failed preparation or launch needs a visible dispatch outcome.

If several rules match one event, determine whether each creates an action or whether a priority policy selects one. Event-ID deduplication prevents redelivery from triggering work again; the multiple-rule policy determines how many actions its first acceptance produces. Define behavior for disabled rules and missing or invalid templates.

Provide match and prompt previews before users authorize a rule. Whether authorization covers subsequent automatic actions or each action needs approval remains open. Stream text is untrusted task input, not authorization or higher-priority instructions. Prompts must reference bounded event content without exposing provider credentials.

### Durable processing and replay

Persist a rule match before external side effects. Deduplicate delivery retries separately from intentional replay. Record the rule revision and resolved action so later edits do not alter queued work.

Receipt and handling are separate facts. Atomically store a newly accepted event and its initial pending processing attempt before acknowledging acceptance. A duplicate provider delivery returns the existing receipt and leaves processing status intact: it neither resets handled work nor removes pending work.

Proposed attempt states are `pending`, `processing`, `handled`, `failed`, and `uncertain`. Persist state transitions, timestamps, outcomes, and errors. Processing claims need cross-process ownership and recoverable leases; an expired claim requires reconciliation with persisted side effects before another worker proceeds.

Workers select pending attempts and skip handled attempts. Record a completed evaluation with no matching rules as handled with a `no_match` outcome. For matched events, retain each rule's action separately; partial success must not cause successful actions to run again when a failed action is retried. The precise handling boundary for matched events remains open: confirmed initial-prompt delivery or confirmed agent-task completion. Neither receipt nor instance creation alone establishes that boundary.

Explicit replay creates a new attempt referencing the same immutable event and preserves previous attempts, statuses, and instance/session links. It must not require the provider to invent a new event ID or clear deduplication records. Preview the chosen rules and side effects before authorized execution; choose whether replay uses the original rule snapshot or current rules explicitly. Repeated submission of one replay request must resolve to the same attempt.

Retry and replay are different operations. Retry reconciles or resumes an existing failed action with its assigned instance identity; replay intentionally allows fresh actions and instances. Uncertain external outcomes need reconciliation or explicit user resolution before either operation can safely repeat the affected side effect.

The implemented inbox records pending and manually handled attempts. Replay requires the current attempt to be handled, then creates an independently identified pending attempt without erasing history. Automated processing, failed/uncertain action states, rule snapshots, and dispatch-side replay semantics remain pending.

Define behavior for concurrent instance creation, provisioning and agent-launch failures, and restart recovery. Apply concurrency limits and loop prevention before automatic execution. An uncertain prompt-send result must remain uncertain rather than being blindly retried.

Redelivery of an accepted event ID must not create a new processing attempt. Local replay should default to inspection or match preview; deliberate execution needs explicit authorization and a new attempt identity. Retention must preserve enough ingestion and dispatch identity to prevent old redeliveries from becoming new work. Replay requires a retained payload; compact deduplication receipts can still prevent duplicate handling after payload expiration.

## Fit with Tandem

All provider lifecycle, ingestion, storage, rules, and dispatch operations enter through `AppService`. TUI, CLI, and MCP remain adapters. Provider runtime and ingestion I/O belong in environment adapters; typed events and rules belong in domain stores.

SQLite stores durable events, attempts, provider credentials, launch snapshots/errors, and notification queues in the private application database. Rule revisions and dispatch state are future tables. Production volume and retention need validation.

Tandem initializes an application service per process. Provider lifecycle uses namespace/package locks and positive Docker ownership; the detached sidecar holds a namespace lease with identity/readiness verification. Cross-process ownership for future rule evaluation and dispatch remains to be designed before background execution.

Provider containers should have ownership separate from instance containers. Stopping or deleting a target instance must not implicitly stop unrelated collection. Reuse established ownership and lifecycle safeguards without assuming an application instance is the right provider abstraction.

Provider networking must not accidentally expose the existing loopback MCP endpoint. Keep credentials out of shared definitions, event records, TUI output, and logs. Trusted Docker builds are not a sandbox; privileges and host mounts need explicit boundaries.

Update runtime agent guidance when the approved provider-authoring and automation workflow is defined. Describe guarantees separately from provider conventions.

## Delivery checklist

Checked tasks have implementation and verification evidence. Unchecked tasks remain work; mixed tasks are split so partial delivery is visible. Phase 3 is the next feature slice. Catalog migration and real-source contract validation are independent follow-ups, not claims of completed automation.

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
- [x] Implement stable provider-scoped event IDs, atomic HTTP batch acceptance, conflict rejection, and persistent provider checkpoint/retry behavior.
- [x] Implement authenticated polling/acknowledgment for durable `received`, `replayed`, and `acknowledged` feedback.
- [x] Implement sidecar worker leases/identity probes, package-scoped Docker ownership, private atomic credential files, and read-only token mounts.
- [x] Enforce 64 KiB events, 1 MiB requests, 100-event batches, and bounded feed/history projections.
- [ ] Validate richer profile semantics against real source observations, including edits, deletions, ordering, and context completeness.
- [ ] Confirm and implement the `assigned` transition with dispatch.
- [ ] Specify production rate/backpressure and retention policies.

- [ ] **Checkpoint:** real Slack, Jira, and polled-system examples map into v1 or an approved extension without losing inspection or routing information.

### 2. Run providers and inspect their events — complete

- [x] Validate provider packages and manage approved Start, Stop, Pause, Resume, and bounded Logs through `AppService`, TUI, CLI, and MCP.
- [x] Deliver four Docker sample providers with normalized payloads, shared context/metadata, durable delivery, and feedback consumption.
- [x] Provision credentials, start/verify the owned sidecar, build images, save launch snapshots, and verify collector runtime state without manual shell setup.
- [x] Preserve checkpoint volumes and events across Stop/Start; freeze and resume collection without rebuilding.
- [x] Persist pending attempts atomically with events, manually acknowledge handled attempts, and replay handled events with idempotent request identities and preserved history.
- [x] Render Events second and Providers third with profile-specific rows, runtime/error state, details, logs, selection preservation, and exact source links/filters.
- [x] Verify source and sidecar restarts preserve accepted events/checkpoints; recover the sidecar at its recorded address without duplicate ownership.
- [x] Stop an owned installation after its package directory is removed using retained identity and fresh Docker evidence.
- [x] Document the implemented provider-authoring/lifecycle and event protocol, including approval, credentials, ownership, and recovery boundaries.

- [x] **Automated checkpoint:** four profiles deliver 40 distinct Docker events with acknowledged feedback; redelivery/restart retain identities; adapter tests preserve handled/replay history; lifecycle tests prove Pause/Resume, Stop/Start, sidecar recovery, Logs, and Stop after package deletion.

### 3. Preview rules against normalized events — pending

- [ ] Add declarative conditions over normalized fields and metadata, with each rule selecting a template and initial prompt.
- [ ] Preview matches and rendered prompts without creating instances or contacting agents.
- [ ] Resolve rule authorization, multiple matches, template validation, handling-completion boundary, and rule enablement/replay policy.

- [ ] **Checkpoint:** representative Slack and Jira events match the intended rules and display the selected template and resolved initial prompt.

### 4. Create a fresh instance and start agent work — pending

- [ ] Persist versioned rule matches and action identities before creating fresh instances through `AppService`.
- [ ] Deliver the initial prompt to a new conversation at the agreed preparation/readiness gate, under the chosen authorization policy.
- [ ] Add bounded concurrency, loop controls, and visible provisioning, launch, and uncertain-send outcomes.
- [ ] Link events to instances and sessions; verify duplicate delivery and worker recovery cannot silently create repeated work.
- [ ] Persist handled/no-match outcomes and per-action failures; verify handled attempts are skipped, retries preserve successful actions, and authorized replay records fresh actions separately.
- [ ] Persist `assigned` notifications atomically with accepted instance assignment and deliver them to the originating provider with retry and deduplication support.

- [ ] **Checkpoint:** a matching event creates a fresh instance from its rule's template, starts its initial prompt, and produces correlated assignment feedback to its provider; notification retries preserve the assignment, and explicit replay records a distinct action and notification.

### 5. Prove sustained operation — developer recovery verified, production work pending

- [x] Verify distinct batch events are retained, identical redelivery is deduplicated, conflicting content rolls back the batch, and receipt loss preserves identical provider retry payloads.
- [x] Verify durable notification retry/acknowledgment and sample-provider notification-ID deduplication.
- [ ] Exercise sustained real-source volume, collector health/source freshness, production backpressure, history retention, and arbitrary log/payload secret handling.
- [ ] Verify historical imports and deliberate replay through the future rule/dispatch authorization policy.
- [ ] Prevent feedback-driven source-system update loops and verify external side-effect reconciliation.
- [ ] Document and verify rule authoring, instance mapping, dispatch, and replay through MCP.

- [ ] **Checkpoint:** a chosen real source runs unattended within its limits, and users can explain every automated action from retained event and dispatch records.

### Verification evidence

- `cargo test --all-targets`: 555 passed, 4 ignored at the last full verification.
- `python3 -m unittest discover -s projects-generators/tests -p 'test_*.py'`: 12 passed.
- `cargo clippy --all-targets -- -D warnings`, `cargo check --release`, formatting, and diff checks passed.
- `python3 projects-generators/tests/events_smoke.py --docker`: four profiles, 40 durable events, feedback acknowledgment, deduplication, and sidecar restart passed.
- `python3 projects-generators/tests/providers_smoke.py`: discovery, automatic setup, four managed collectors, Pause/Resume, Logs, checkpoint-preserving Stop/Start, sidecar recovery, and template-independent Stop passed.

These are implementation-time results; rerun relevant checks after subsequent edits. Interactive TUI behavior is covered by terminal-backend tests, including narrow layouts, confirmation, tab focus, metadata inspection, and exact provider/event links.

## Decisions for iteration

1. **Shared context and profile fields:** what should Tandem normalize for people, attachments, relations, threads, work-item changes, and operational signals?
2. **Context delivery:** provider-supplied snapshots or later authorized resolution of thread context and attachments?
3. **Authorization:** authorize automatic execution when enabling a rule, or approve each matched action?
4. **Prompt gate:** start after workspace preparation or wait for full instance readiness?
5. **Multiple matches:** one action per matching rule or a priority-based choice?
6. **Provider packaging:** Docker image/build only, or Compose recipes for providers needing multiple services?
7. **Dispatch ownership:** how should future rule evaluation and dispatch acquire cross-process ownership and recover uncertain outcomes? Provider HTTP transport and sidecar ownership are implemented.
8. **Rule enablement and replay:** prospective matches only, or an explicit scan of already accepted events when enabling a rule?
9. **Concrete example:** which event, condition, template, and initial prompt should prove the first end-to-end workflow?
10. **Handled boundary:** does a matched action become handled when its initial prompt is confirmed delivered, or when the agent task is confirmed complete?
11. **Replay rules:** evaluate current rules or rerun the original rule snapshots, and how should users select the scope of a replay?
12. **Assignment feedback:** confirm the assignment boundary, outcomes, and correlation with rule revisions/actions; extend the implemented polling queue.
