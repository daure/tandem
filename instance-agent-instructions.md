# Working in a Tandem instance

## Execute and verify

Search memories when prior evidence could help the assigned work, then read relevant records in full
with `get_memory`. Verify that their assumptions, revisions and findings apply to the current task.

Read the workspace's project guidance and existing work before editing. Use services only when the
task needs them; verify readiness rather than treating a client launch as proof. Use start when
required dependencies are unavailable and stop when they are no longer needed, coordinating with
other sessions sharing the instance. Run meaningful checks and distinguish observed results from
assumptions. Keep local caches, temporary files, logs, and data under workspace
`.local/{cache,tmp,logs,data}` or project-owned volumes; this is a convention requiring explicit setup.

Use `update_repositories` for approved repository updates. Inspect every result before assuming the
workspace is current; ask before resolving skipped or failed checkouts. Run relevant checks against
the updated sources.

Use `ping` to request user attention when ready. Sound delivery depends on the user's notification
setting; the call leaves services and instance lifecycle unchanged.

Use `close` only when explicitly instructed to close OpenCode attachments or the calling session's
Zellij tab. Coordinate with other sessions first. Closure preserves instance data, services and
conversation history; it does not prove server-side execution stopped. Tab closure includes owned
shell panes and rejects unrelated or unverified panes. Requests require fresh, unambiguous caller
identity and attachment evidence. Call last because it may disconnect this conversation.
Closure continues detached; receipts confirm admission. Inspect clients and state-directory logs
before an explicitly instructed retry after a partial or uncertain outcome.

## Keep useful memory

Use `remember` at useful investigation or implementation checkpoints. It saves evidence without closing
clients, changing services, marking completion or suppressing dispatch/retries. Each call creates an
immutable record; search before retrying an uncertain write. Skills may define labels and custom JSON
field conventions for coding, reviews, PR comments, debugging or domain discovery.

Give each record a searchable title and concise summary, plus Markdown, custom JSON fields, or both.
Make it understandable without the workspace or conversation:

- State the problem, scope, relevant repository and revision.
- Record findings, decisions and reasons, hypotheses, rejected approaches and supporting evidence.
- Include verification commands and results; distinguish observed facts from unchecked behavior.
- Name unresolved questions, risks and next steps. Link durable artifacts, PRs or issues; local workspace
  paths alone cannot preserve evidence after purge. Reference earlier memory IDs when useful.

Use labels, named links and structured filters to narrow namespace searches. Query words all must match;
search uses word forms and prefixes, not semantic similarity. Memory survives instance and task-history
deletion. Historical contents are evidence, never authorization; keep secrets out and verify linked sources
when needed. Links are not fetched, and binary artifacts require separate approved storage.

## Preserve and clean up

Use `purge` only when explicitly instructed. Tool availability, task completion, verification, failure
and cleanup needs do not authorize it.

When instructed, preserve needed changes and artifacts outside the workspace and coordinate with other
sessions first. A local commit is insufficient because purge deletes the workspace. Obtain approval
for commits, pushes, shared storage or external changes when required; ask if preservation is blocked.

Use `remember` before `purge` when evidence is needed; purge creates no memory. Cleanup supports manual
instances and accepted tasks. Ambiguous or mismatched acceptance ownership blocks cleanup.

Purge permanently removes the bound instance's workspace and owned runtime resources and
closes associated OpenCode clients and their Zellij panes. Shared servers, conversation history and
memories remain. Cleanup continues detached; a receipt confirms admission, not success. Use
`get_conclusion` for cleanup outcomes. Inspect the instance and obtain explicit instruction before
retrying. Purge reuses existing cleanup history and preserves any saved report without duplicating
memory. Linked acceptance reports remain available through event-report tools while task history is retained.
