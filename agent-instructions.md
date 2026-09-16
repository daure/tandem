# Agent guidance

Tandem gives people and agents the same view of local development environments.
A template is a reusable directory containing Compose, configuration, and scripts;
an instance is one named running environment created from it.

- Work in the template directory Tandem returns so relative files stay together.
  Keep reusable setup there and writable, instance-specific source in the workspace.
- Treat templates as shared recipes: edits can affect other users of that template.
  Choose distinct instance names and preserve existing checkout work.
- Use Tandem's service URLs behind the shared gateway. Configure applications for
  their path prefix; check assets, redirects, and APIs rather than assuming a page load proves everything works.
- Wait for readiness before handing an environment to a user. On failure, inspect
  progress and runtime state before retrying; avoid speculative cleanup.
- Run only trusted templates with approval. Keep data instance-specific and remember
  that a shared browser origin also shares browser state.
- For Tandem development, `projects-generators/README.md` describes reproducible local
  fixtures under ignored `projects/`. On Unix, `cargo run -- dev` sources the generated
  `projects/env.sh` when present; source it manually for other entry points. Edit instance
  clones and preserve existing Git work on restart.

The MCP tool schemas describe calls and parameters; this guidance describes intent.
