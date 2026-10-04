# Working in a Tandem instance

## Execute and verify

Search prior acceptance reports when their evidence could help the assigned work, then retrieve
relevant reports in full. Verify that their assumptions and findings apply to the current task.

Read the workspace's project guidance and existing work before editing. Use services only when the
task needs them; verify readiness rather than treating a client launch as proof. Use start when
required dependencies are unavailable and stop when they are no longer needed, coordinating with
other sessions sharing the instance. Run meaningful checks and distinguish observed results from
assumptions. Keep local caches, temporary files, logs, and data under workspace
`.local/{cache,tmp,logs,data}` or project-owned volumes; this is a convention requiring explicit setup.

## Conclude accepted work

Conclude event-assigned work only after the assigned objective is successfully completed and verified.
If work fails, remains incomplete, or is blocked, preserve the instance and report the blocker to the user.
Before concluding, preserve artifacts needed after purge in an approved durable location. A local
commit inside this workspace is insufficient because the workspace will be deleted. Obtain approval
for commits, pushes, shared storage, or external changes when required; ask if preservation is blocked.

Supply a searchable title, a concise outcome summary, and full Markdown contents rather than a file
path. Make the report useful without this workspace or conversation:

- State the triggering problem, scope, and relevant context.
- Explain the investigation, evidence, decisions, and work performed in enough detail to reproduce
  the reasoning; distinguish verified facts from hypotheses.
- Record the outcome and verification, including commands, results, failures, and unchecked behavior.
- Identify preserved changes and artifacts by durable location or revision, and state remaining risks
  and follow-up work. Do not rely on links to files that purge will remove.

Report honestly: saving a conclusion is not proof of success. Use the saved report's cleanup state
to check purge completion from another instance connection if this client closes. Workspaces without
a retained acceptance use stop for resource release and an approved management operation for deletion.
