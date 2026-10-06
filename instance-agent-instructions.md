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

Use `update_repositories` for approved repository updates. Inspect every result before assuming the
workspace is current; ask before resolving skipped or failed checkouts. Run relevant checks against
the updated sources.

## Conclude

The `conclude` tool controls the owning instance, including manually created instances.
Ignore `conclude` unless explicitly instructed to call it. Tool availability, task completion,
verification, failure, and resource cleanup needs are not instructions to call it.

When instructed to call it, preserve artifacts needed after purge in an approved durable location.
A local commit inside this workspace is insufficient because the workspace will be deleted. Obtain approval
for commits, pushes, shared storage, or external changes when required; ask if preservation is blocked.

An instructed call supplies a searchable title, a concise outcome summary, and full Markdown contents
rather than a file path. Make the report useful without this workspace or conversation:

- State the triggering problem, scope, and relevant context.
- Explain the investigation, evidence, decisions, and work performed in enough detail to reproduce
  the reasoning; distinguish verified facts from hypotheses.
- Record the outcome and verification, including commands, results, failures, and unchecked behavior.
- Identify preserved changes and artifacts by durable location or revision, and state remaining risks
  and follow-up work. Do not rely on links to files that purge will remove.

The call permanently purges the instance's workspace and owned runtime resources and closes associated
OpenCode clients and Zellij panes. A retained triggering acceptance receives an immutable report before
cleanup. Without an acceptance link, report contents are not saved; preserve needed evidence elsewhere.
Ambiguous or mismatched acceptance ownership blocks conclusion. Shared servers and conversation history
remain. Report the actual outcome honestly; saving a conclusion is not proof of success. Cleanup continues
after disconnection. Saved reports record cleanup outcomes and survive failed cleanup; inspect the instance
to verify unlinked cleanup. Retrying requires explicit instruction and instance inspection, with identical
contents when a report was saved.
