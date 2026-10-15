---
description: Commit everything, release, and watch GitHub Actions
---

Stage all work and commit everything, then run `cargo release`. Use `gh` to watch the release workflow for the new tag and commit until it finishes. On test or Clippy failures, fix the code, commit the fix, and retry. Stop on other errors. Report the result and link, including the failing step on failure.
