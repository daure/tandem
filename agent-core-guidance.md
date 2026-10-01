# Tandem placement baseline

Templates are shared, Git-managed recipes. Apply this baseline together with editable guidance;
ask the user before acting when they conflict.

- Keep only team-shareable, nonsecret recipes, static assets, scripts, and seed files in templates.
  Relative static configuration paths and build contexts are allowed. Keep caches, temporary files,
  logs, databases, dependencies, and build outputs outside templates, including ignored directories
  and `tandem-files/`.
- Put instance-local mutable files in the instance workspace, conventionally under hidden
  `.local/cache`, `.local/tmp`, `.local/logs`, or `.local/data`, or in project-owned named volumes.
  This layout is a template convention; configure tools and mounts explicitly, check existing paths,
  and prepare writable directories with ownership suitable for the container user. Use project-scoped
  volumes without fixed global names or `external` unless sharing is approved. Before an instance
  exists, use a private host temporary directory outside templates and repositories.
- Use shared caches only after user approval of their location, permissions, and retention policy.
  Keep them outside templates and identify who owns cleanup.
- Keep secrets in private external files outside templates and Git repositories, even when paths
  are ignored or used as seeds. Reference those files without copying secrets into shared assets.
  Avoid secret values in Compose interpolation or environment declarations because resolved configuration
  is retained locally; prefer private file-backed secrets or read-only external mounts.
- Mount template assets read-only. For mutable seeds, use workspace copies and write to those copies.
- Tandem rejects writable binds and local bind-backed volumes that expose templates, including aliases
  and parent directories, and local build-cache exports into templates. Existing template assets may be
  mounted read-only; create missing assets while authoring the recipe. Privileged containers and externally
  configured Docker resources still require trust.
- Set host cache environment variables to host absolute paths and container cache variables to
  container-visible paths; configure mounts when needed. For Bun, set `BUN_INSTALL_CACHE_DIR` to an
  instance-local cache path. Its download cache does not relocate `node_modules` or build outputs;
  place those separately.
- Stop preserves instance data. Delete removes the instance workspace and owned volumes;
  external or shared storage requires its owner's cleanup policy.
- Preserve Tandem-owned runtime records and use lifecycle tools for cleanup; ownership evidence
  protects user data from unverified deletion.
