import { test } from "node:test"
import assert from "node:assert/strict"
import { chmod, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { promptFreshSession } from "../session-input.mjs"

function api(prompt) {
  return {
    server: "http://127.0.0.1:4199",
    client: { session: {
      create: async () => ({ data: { id: "ses_created" } }),
      promptAsync: prompt,
    } },
    ui: {},
    tabs: { enabled: () => true, focus: () => true },
  }
}

test("fresh empty sessions return identity without submitting model input", async () => {
  const outcome = await promptFreshSession(api(() => assert.fail("empty sessions have no model input")), { directory: "/work/review" }, new AbortController().signal)
  assert.deepEqual(outcome, { session_id: "ses_created", server: "http://127.0.0.1:4199", prompt_outcome: "not_submitted", error: null })
})

test("session identity reaches the private receipt before prompt delivery", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "tandem-origin-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const path = join(root, "identity.json")
  await writeFile(path, JSON.stringify({ request_id: "request-1", directory: "/work/review" }), { mode: 0o600 })
  let attempts = 0
  const outcome = await promptFreshSession(api(async () => {
    attempts++
    assert.deepEqual(JSON.parse(await readFile(path, "utf8")), { request_id: "request-1", directory: "/work/review", session_id: "ses_created", server: "http://127.0.0.1:4199" })
    throw new Error("delivery response lost")
  }), { directory: "/work/review", initial_prompt: "Review the message", identity_receipt: path }, new AbortController().signal)
  assert.equal(attempts, 1)
  assert.equal(outcome.session_id, "ses_created")
  assert.equal(outcome.prompt_outcome, "uncertain")
  assert.equal(outcome.creation_outcome, "created")
})

test("correlated creation distinguishes preflight refusal from lost native creation", async (t) => {
  for (const failure of ["preflight", "creation"]) {
    await t.test(failure, async () => {
      const bridge = api(() => assert.fail("failed creation cannot submit input"))
      let attempts = 0
      bridge.client.session.create = async () => {
        attempts++
        throw new Error("creation response lost")
      }
      if (failure === "preflight") bridge.ui.selectedModel = () => undefined
      const outcome = await promptFreshSession(bridge, {
        directory: "/work/review",
        initial_prompt: "Review the message",
        identity_receipt: join(tmpdir(), "unused-identity.json"),
      }, new AbortController().signal)
      assert.equal(outcome.session_id, null)
      assert.equal(outcome.creation_outcome, failure === "preflight" ? "not_created" : "uncertain")
      assert.equal(attempts, failure === "preflight" ? 0 : 1)
    })
  }
})

test("identity handoff rejects symlinks, public files and mismatched workspaces", async (t) => {
  for (const failure of ["symlink", "public", "workspace"]) {
    await t.test(failure, async (t) => {
      const root = await mkdtemp(join(tmpdir(), "tandem-origin-"))
      t.after(() => rm(root, { recursive: true, force: true }))
      const path = join(root, "identity.json")
      const target = failure === "symlink" ? join(root, "target.json") : path
      await writeFile(target, JSON.stringify({ request_id: "request-1", directory: failure === "workspace" ? "/other" : "/work/review" }), { mode: 0o600 })
      if (failure === "symlink") await symlink(target, path)
      if (failure === "public") await chmod(path, 0o644)
      const outcome = await promptFreshSession(api(() => assert.fail("unverified identities cannot submit input")), { directory: "/work/review", initial_prompt: "Review the message", identity_receipt: path }, new AbortController().signal)
      assert.equal(outcome.session_id, "ses_created")
      assert.equal(outcome.prompt_outcome, "not_submitted")
      assert.match(outcome.error, /setup failed/)
    })
  }
})
