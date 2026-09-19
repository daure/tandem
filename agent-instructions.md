# Using Tandem

A template is a shared Compose recipe; an instance has its own name and workspace.

## Templates

- Edit `compose.yaml` in the returned template directory; keep reusable scripts beside it.
  Template edits are shared across instances; keep application edits in instance workspaces.
- Use `manifest_schema` from the `get_instructions` response to construct `tandem.json`.
  Save complete manifests through
  `update_template_manifest` with approval to change shared configuration. Validation preserves the file
  on failure; Compose compatibility and repository access are checked at startup.
  Reread templates after direct filesystem edits.
- `compose.yaml` defines services and dependencies. Optional `tandem.json` declares `repositories`,
  setup services in `one_shots`, and service-keyed `routes` with internal `port`,
  `strip_prefix`, relative `readiness_path`, and nonempty `readiness_contains`.
- Optional `tandem-agents.md` beside `compose.yaml` supplies template-specific workspace guidance.
  Its UTF-8 contents (up to 256 KiB) are appended verbatim to generated workspace `AGENTS.md`,
  including literal `{{...}}`. The file must resolve within the template directory. Read errors block
  generation, and edits apply to future generation; existing workspace guidance is preserved.
- Generated workspace guidance supplies the instance title, service/repository/code-path mappings,
  repository guidance references, generic Compose commands, gateway URLs, and a general self-verification
  mandate. Add template-specific procedures rather than repeating that content: how applications are
  compiled or built, whether hot reload is configured and which changes require rebuilding or restarting,
  how logs are captured and viewed, and how to verify changes with expected results. Specify
  host/container execution locations, working directories and prerequisites. Ensure the template exposes
  usable logs; generic Compose logs commands alone do not guarantee application log access. Start with
  `## Title` and use deeper headings for subsections.
- Write `tandem-agents.md` for development, debugging, navigation, and self-verification. Document verified template-specific facts: reload/build and restart behavior; host or container commands, working directories, prerequisites, and data access; executable tests or a smoke check with expected results; and service-specific logs, inspection, lifecycle actions, and hazards needed to diagnose failures. Refer to generated service mappings, gateway URLs, and project-scoped Compose controls. Do not repeat repository guidance, generic Tandem operations, or MCP schemas; verify every command and path against the template.
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
  startup. With Branch instances enabled, it checks out the instance-named remote branch when present,
  otherwise creates a local branch from the remote default; when disabled it uses the default branch.
  Local source defaults follow their checked-out branch. New branches are not pushed.
- Existing checkout roots and exact origin URLs must match; their branches and work are preserved.
  Git updates require explicit user approval. Failed provisioning stops startup; completed checkouts
  survive retries. Resolve mismatches explicitly; never delete or reset user work to make a retry pass.
- Provisioning requires Linux, host Git with `switch` support, and noninteractive credentials;
  SSH requires known hosts with strict host-key checking. Configure Compose workspace mounts/build
  contexts, dependencies, migrations and app-specific setup.
  Declare app setup jobs in `one_shots` and gate dependents on successful completion.
  Do not add clone scripts or `repo-sync` services for declared repositories.
- Templates without declared repositories own their checkout workflow. When declaring existing sources
  and targets, remove clone services and their Compose dependencies; retain app setup jobs.

## Operations

- Tandem writes workspace-root `AGENTS.md` before container startup and generates it if missing before
  opening a workspace. Read it and the repository guidance it lists for service/repository mappings,
  scoped Compose commands and gateway URLs. Inspect containers for missing mappings and runtime state
  for current health. The base template at `workspace_agents_template` affects future generation;
  bundled updates replace it after backing up local edits.
- Workspace open and close commands are shared within a Tandem home and execute through host `sh -c` in the
  workspace. Configure and run only trusted commands with approval; quote `"$TANDEM_WORKSPACE"`
  and `"$TANDEM_INSTANCE"` when using them.
  MCP deletion and purges run the saved close command before removing each existing workspace, after
  Docker cleanup; CLI `delete-instance` opts in with `--close-command` or `-cc`, including headless runs.
  Empty disables it; failures and ten-second timeouts produce operation warnings while
  deletion continues within its cleanup deadline. Use repeat-safe commands for deletion retries.
- Run trusted templates with approval: repository provisioning uses host Git and credentials;
  Docker execution grants local privileges.
  Keep environments local; gateway routes share a browser origin.
- Stop preserves instance data; deletion permanently removes its workspace and owned resources.
  Preserve Tandem-generated `.tandem-*` files for ownership checks and cleanup.
- Inspect `list_instances` for runtime evidence and retained failures. Running is not proof of health;
  Docker healthchecks and gateway content readiness are separate checks.
- Restart and service start/stop require approval and use existing containers with their current
  configuration and data; they exclude setup jobs and the gateway. Service actions affect only the
  named service, leaving dependencies untouched. These actions do not build or apply template edits.
  Restart and service start verify running state, configured healthchecks and gateway content readiness;
  routed services need current template readiness configuration. Services without checks are verified
  only as running. Unpause selected containers before starting or restarting.
- Template mutation tools are blocked while instances start from that template.
- Verify readiness before handing over URLs. On failure, inspect operation output, containers,
  and logs before retrying; setup jobs may rerun. After reconnecting, inspect runtime inventory
  because operation history is process-local.
