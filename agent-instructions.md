# Using Tandem

A template is a shared development recipe; an instance has its own name and workspace.
Read `core_guidance` together with this editable guidance for placement and cleanup decisions.

## Templates

- All templates live under `templates_root`. Keep this directory
  under Git version control. Obtain approval before
  initializing a repository, committing, or pushing.
- Edit template files in the returned directory; keep reusable scripts there.
  Template edits are shared across instances; keep application edits in instance workspaces.
  New templates contain only `tandem.json` with `{}`. Blank templates prepare workspace-only instances
  with standard `AGENTS.md` guidance; add repositories, Compose, routes, or template guidance as needed.
- Use `manifest_schema` to construct `tandem.json`.
  Save complete manifests through
  `update_template_manifest` with approval to change shared configuration. Validation preserves the file
  on failure; Compose compatibility and repository access are checked at startup.
  Reread templates after direct filesystem edits.
- `compose.yaml` defines services and dependencies. Optional `tandem.json` declares `repositories`,
  setup services in `one_shots`, and service-keyed `routes` with internal `port`,
  `strip_prefix`, relative `readiness_path`, and nonempty `readiness_contains`.
  Compose-only templates use empty manifest defaults. Repository-only templates omit `compose.yaml`
  and declare at least one repository in `tandem.json`; routes and one-shots require Compose.
  Guidance-only templates need only `tandem-agents.md`: creation prepares a workspace and `AGENTS.md`
  without Git or Docker. Their manifest is optional. An instance retains its execution kind;
  use a new name when switching between workspace-only and container-backed execution.
- Optional `tandem-agents.md` in the template directory supplies template-specific workspace guidance.
  Its UTF-8 contents (up to 256 KiB) are appended verbatim to generated workspace `AGENTS.md`,
  including literal `{{...}}`. The file must resolve within the template directory. Read errors block
  generation, and edits apply to future generation; existing workspace guidance is preserved.
- Optional `tandem-files/` supplies workspace seed files. Its contents are copied into the workspace
  root before workspace guidance generation, any creation-requested OpenCode client launch,
  and repository provisioning.
  Nested directories, hidden files, binary contents, and file permissions are supported; paths must
  be UTF-8 and entries must be real directories or regular files. Symlinks and special files are refused.
  Keep repository targets clear and reserve root `AGENTS.md`, root `.tandem-*`, and every `.git` path
  for Tandem or Git. Existing regular files are preserved on preparation retries; incompatible
  destinations block preparation. Template edits affect future preparation, not ready workspaces.
  Files-only templates need no manifest, Git, or Docker.
- Generated workspace guidance includes repository instructions when repositories are identified,
  Compose controls when services are identified, and HTTP testing instructions when URLs exist.
  Write `tandem-agents.md` for verified template-specific development and self-verification:
  build/reload behavior, host or container commands, working directories, prerequisites, tests or smoke
  checks with expected results, data access and diagnostic hazards. Service templates must expose usable
  logs; generic Compose commands alone do not guarantee application log access. Refer to generated
  mappings and controls rather than repeating them, repository guidance, or MCP schemas. Start with
  `## Title` and use deeper headings for subsections.
- Route HTTP through Tandem's gateway at `/<instance>/<service>/`; only the gateway publishes ports.
  Prefix stripping affects incoming requests; verify browser assets, redirects, and API paths too.
  Leave `traefik.*`/`io.tandem.*` labels to Tandem; omit `container_name` and `profiles`.
- Compose receives `TANDEM_INSTANCE`, `TANDEM_WORKSPACE`, `TANDEM_ORIGIN`, and Unix `TANDEM_UID`/`TANDEM_GID`.
  Enabling Branch instances also sets `TANDEM_BRANCH` to the instance name.
  Explicitly pass variables needed inside containers through Compose.

## Repositories

- Declare sources and workspace-relative targets in `tandem.json`, for example
  `"repositories": [{"source": "https://github.com/team/app.git", "target": "app"}]`.
  Sources accept absolute local paths, HTTPS URLs, or `ssh://` URLs; use host credentials and
  omit URL passwords/tokens. Targets must be non-overlapping paths of non-hidden directory names
  using letters, digits, dots, underscores or hyphens; the workspace root is not a target.
- Tandem provisions missing repositories with host Git before Compose configuration, builds or
  startup, using depth-1 single-branch clones of the source default. With Branch instances enabled,
  it shallow-fetches and checks out the instance-named remote branch when present,
  otherwise creates a local branch from the remote default; when disabled it uses the default branch.
  Local source defaults follow their checked-out branch. New branches are not pushed.
  Restore history on demand with `git fetch --unshallow` if the checkout is shallow;
  fetch other branches explicitly.
- Existing checkout roots and exact origin URLs must match; their branches and work are preserved.
  Git updates require explicit user approval. Failed provisioning stops startup; completed checkouts
  survive retries. Resolve mismatches explicitly; never delete or reset user work to make a retry pass.
- Provisioning requires Linux, host Git with `switch` support, and noninteractive credentials;
  SSH requires known hosts with strict host-key checking. For service templates, configure workspace mounts/build
  contexts, dependencies, migrations and app-specific setup.
  Declare app setup jobs in `one_shots` and gate dependents on successful completion.
  Do not add clone scripts or `repo-sync` services for declared repositories.
- Templates without declared repositories own their checkout workflow. When declaring existing sources
  and targets, remove clone services and their Compose dependencies; retain app setup jobs.

## Operations

- With OpenCode integration and creation-history cleanup enabled (both default on), creating a new
  instance permanently deletes conversations for its exact workspace path, including history left by
  a deleted instance with the same name. Include this in creation approval. Existing instances and
  provisioning retries retain conversations. Active clients or cleanup failures block creation;
  close clients or ask the user to disable cleanup if they want to preserve history.
- Tandem writes workspace-root `AGENTS.md` after seed copying and before any creation-requested OpenCode
  launch or repository provisioning, and generates it if missing when opening a prepared instance workspace.
  Early guidance uses declared repositories and previewed services. Preparation refreshes verified mappings
  before container startup only when the newly generated file remains unchanged; existing files and client
  edits are preserved. A client's presence does not certify that sources or services are ready.
  Wait for preparation, then reread workspace `AGENTS.md` and the repository guidance it lists.
  For service instances, inspect containers for missing mappings and runtime state
  for current health. The base template at `workspace_agents_template` affects future generation;
  bundled updates replace it after backing up local edits.
- Run trusted templates with approval: repository provisioning uses host Git and credentials;
  Docker execution grants local privileges.
  Keep environments local; gateway routes share a browser origin.
- Instance creation applies the selected template and builds services that declare `build`, using
  Docker's layer cache, before starting containers. Build contexts must contain the intended sources
  and dependencies, including when retrying preparation or reusing a deleted instance name.
- Stop preserves instance data; deletion permanently removes its workspace and owned resources.
  With OpenCode integration enabled, instance purge first closes associated observed clients in the
  workspace and its subdirectories. Closure failures block that instance's deletion; clients outside
  Zellij require manual closure. Include client closure in purge approval. Conversation history and
  shared OpenCode servers are preserved. Clients can close even when later resource deletion fails.
  Preserve Tandem-owned runtime records; use lifecycle tools for cleanup.
  Failed deletion can be retried when containers are absent: cleanup validates the retained instance
   record and local ownership evidence, then checks project membership. Missing execution-kind metadata
  requires Docker access for recovery; unverifiable ownership blocks resource deletion and preserves data.
  A failed new-instance request with no accepted worker or preparation record can be deleted without
  Docker access; only its failure records are removed. Unverified workspaces, clients, and resources
  are preserved.
- Inspect `list_instances` for runtime evidence and retained failures. Running is not proof of health;
  Docker healthchecks and gateway content readiness are separate checks.
  Workspace-only instances (blank, repository-only, or guidance-only) are retained across processes and report
  `Workspace ready` after any provisioning and guidance generation; this does not certify application tests.
  Their lifecycle requires no Docker.
  Stop preserves them without work, and container restart/service actions do not apply. Retry failed
  preparation with instance creation after correcting the cause. A listing with `runtime_error` contains
  workspace-only inventory and retained startup evidence while Docker inventory is unavailable; it is not a complete container listing.
- Restart and service start/stop require approval and use existing containers with their current
  configuration and data; they exclude setup jobs and the gateway. Service actions affect only the
  named service, leaving dependencies untouched. These actions do not build or apply template edits.
  Restart and service start verify running state, configured healthchecks and gateway content readiness;
   routed services use saved launch-time readiness assertions; imported instances without saved assertions
   require current template readiness configuration. Services without checks are verified
  only as running. Unpause selected containers before starting or restarting.
- Template mutation tools are blocked while instances start from that template.
- Verify readiness before handing over URLs. On failure, inspect operation output, containers,
  and logs before retrying; setup jobs may rerun. Startup continues after the initiating client closes.
  Reconnect using the same Tandem home and namespace: `list_instances` shows current runtime and startup
  failures, and `get_operation` retains the latest startup attempt per instance until another startup or
  deletion. Other operation history is process-local. Interrupted startups require an explicit retry;
  reconnecting only observes their state.
