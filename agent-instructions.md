# Using Tandem

A template is a shared Compose recipe; an instance has its own name and workspace.

## Templates

- Edit the returned template directory; keep reusable scripts beside `compose.yaml`.
  Template edits are shared across instances; keep application edits in instance workspaces.
- Use the generated `manifest_schema` returned alongside the editable guidance to construct
  `tandem.json`. Save complete manifests through `update_template_manifest` after approval to change
  shared configuration. It validates manifest rules before atomic replacement and leaves the file
  unchanged on validation failure. It runs no Docker or Git commands; Compose compatibility and
  repository access are checked at startup. Template mutation tools notify running Tandem observers
  sharing the same home. Direct filesystem edits do not publish notifications; reread templates
  explicitly after editing. Files are validated on read.
- `compose.yaml` defines services and dependencies. Optional `tandem.json` declares `repositories`,
  lists app setup service names in `one_shots`, and maps service names to `routes`. Routes specify the internal `port`,
  `strip_prefix`, relative `readiness_path`, and nonempty `readiness_contains`.
- Route HTTP through Tandem's gateway at `/<instance>/<service>/`; only the gateway publishes ports.
  Prefix stripping affects incoming requests; verify browser assets, redirects, and API paths too.
  Leave `traefik.*`/`io.tandem.*` labels to Tandem; omit `container_name` and `profiles`.
- Compose receives `TANDEM_INSTANCE`, `TANDEM_WORKSPACE`, `TANDEM_ORIGIN`, and Unix `TANDEM_UID`/`TANDEM_GID`.
  Branch instances supplies `TANDEM_BRANCH` as the instance name when enabled and controls initial
  branch selection for declared repositories.
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
- Tandem validates existing checkout roots and exact origin URLs, then preserves their branches,
  commits, staged changes and untracked work. Restarting does not fetch, pull, switch or reset them.
  Failed provisioning stops startup; completed checkouts survive retries. Resolve mismatches explicitly,
  never delete or reset user work to make a retry pass.
- Agents own repository declarations, host Git access and noninteractive credentials (including known
  SSH hosts), Compose workspace mounts/build contexts, dependencies, migrations and app-specific setup.
  Declare app setup jobs in `one_shots` and gate dependents on successful completion.
  Do not add clone scripts or `repo-sync` services for declared repositories.
- Templates without repository declarations retain their own checkout workflow. To migrate, declare
  the same sources and targets, remove clone services and their Compose dependencies, and retain app
  setup jobs. Git updates to existing workspaces remain explicit user-approved operations.

## Operations

- The workspace open command is shared by processes using the same Tandem home. Configure only
  trusted commands with user approval; saving does not execute them. `run_open_command` runs the
  saved command for a named instance workspace after approval. Workspace opening runs the command
  through `sh -c` on the host with the workspace as its working directory and in
  `TANDEM_WORKSPACE`; its instance name is in `TANDEM_INSTANCE`. Use `"$TANDEM_WORKSPACE"` to preserve spaces and shell characters. An empty
  command uses the system folder opener. The launching process supplies inherited session variables.
- Run trusted templates with approval: repository provisioning uses host Git and credentials;
  Docker execution grants local privileges. Native repository provisioning requires Linux and Git
  with `switch` support; SSH access uses strict host-key checking without interactive prompts.
  Keep environments local; gateway routes share a browser origin.
- Stop preserves instance data; deletion permanently removes its workspace and owned resources.
   Preserve Tandem-generated `.tandem-*` files for ownership checks and cleanup.
- Restart requires approval and restarts existing containers, including stopped ones, with their
  current configuration and data. Instance restart skips one-shot setup jobs; service restart affects
  only that instance's named service. The shared gateway stays untouched. It performs no builds,
  template application or dependency orchestration. Completion requires all targeted containers to
  run and pass configured healthchecks and gateway content assertions within a ten-minute budget.
  Routed services require their current template readiness configuration; services without healthchecks
  or routes are verified only as running. Unpause selected containers before restarting.
- Distinct instances can start concurrently from the same template. Shared gateway setup waits
  within each startup deadline; template mutations remain blocked while starts are active.
- Verify readiness before handing over URLs. On failure, inspect operation output, containers,
  and logs before retrying; setup jobs may rerun. After reconnecting, inspect runtime inventory
  because operation history is process-local.
