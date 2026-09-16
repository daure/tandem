# Using Tandem

A template is a shared Compose recipe; an instance has its own name and workspace.

## Templates

- Edit the returned template directory; keep reusable scripts beside `compose.yaml`.
  Template edits are shared across instances; keep application edits in instance workspaces.
- `compose.yaml` defines services and dependencies. Optional `tandem.json` lists setup service
  names in `one_shots` and maps service names to `routes`. Routes specify the internal `port`,
  `strip_prefix`, relative `readiness_path`, and nonempty `readiness_contains`.
- Route HTTP through Tandem's gateway at `/<instance>/<service>/`; only the gateway publishes ports.
  Prefix stripping affects incoming requests; verify browser assets, redirects, and API paths too.
  Leave `traefik.*`/`io.tandem.*` labels to Tandem; omit `container_name` and `profiles`.
- Compose receives `TANDEM_INSTANCE`, `TANDEM_WORKSPACE`, `TANDEM_ORIGIN`, and Unix `TANDEM_UID`/`TANDEM_GID`.
  Branch instances supplies `TANDEM_BRANCH` as the instance name when enabled.
  Explicitly pass variables needed inside containers through Compose.

## Repositories

- `repo-sync` is a template-defined convention. Declare setup services in `one_shots` and make
  dependent services wait for `service_completed_successfully` in Compose.
- Clone into the instance workspace. Make setup repeatable; preserve existing branches, commits,
  and uncommitted work. Fetching or changing existing checkouts requires an explicit decision.
  Template scripts own branch selection; inspect their handling of `TANDEM_BRANCH` before starting.

## Operations

- The workspace open command is shared by processes using the same Tandem home. Configure only
  trusted commands with user approval; saving does not execute them. `run_open_command` runs the
  saved command for a named instance workspace after approval. Workspace opening runs the command
  through `sh -c` on the host with the workspace as its working directory and in
  `TANDEM_WORKSPACE`; its instance name is in `TANDEM_INSTANCE`. Use `"$TANDEM_WORKSPACE"` to preserve spaces and shell characters. An empty
  command uses the system folder opener. The launching process supplies inherited session variables.
- Run trusted templates with approval: Docker execution grants local privileges.
  Keep environments local; gateway routes share a browser origin.
- Stop preserves instance data; deletion permanently removes its workspace and owned resources.
  Preserve Tandem-generated `.tandem-*` files for ownership checks and cleanup.
- Verify readiness before handing over URLs. On failure, inspect operation output, containers,
  and logs before retrying; setup jobs may rerun. After reconnecting, inspect runtime inventory
  because operation history is process-local.
