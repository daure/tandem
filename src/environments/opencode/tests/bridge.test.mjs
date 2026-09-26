import { test } from "node:test"
import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import plugin from "../bridge.mjs"

test("presence follows the displayed conversation, home route, and client disposal", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "tandem-bridge-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  t.mock.timers.enable({ apis: ["setInterval"] })
  let dispose
  let route = { name: "session", params: { sessionID: "ses_one" } }
  let activity = "busy"
  const api = {
    route: { get current() { return route } },
    state: {
      ready: true,
      path: { directory: "/work/review" },
      session: {
        get: (id) => ({ title: `Title ${id}`, directory: "/work/review" }),
        status: () => ({ type: activity }),
      },
    },
    lifecycle: { onDispose: (fn) => { dispose = fn } },
  }
  const file = join(root, `${process.pid}.json`)
  const receipt = async () => JSON.parse(await readFile(file, "utf8"))
  await plugin.tui(api, { presenceDirectory: root })
  assert.equal((await receipt()).id, "ses_one")
  assert.equal((await receipt()).activity, "busy")
  route = { name: "session", params: { sessionID: "ses_two" } }
  activity = "idle"
  t.mock.timers.tick(1000)
  await waitFor(async () => (await receipt()).id === "ses_two")
  assert.equal((await receipt()).activity, "idle")
  route = { name: "home" }
  t.mock.timers.tick(1000)
  await waitFor(async () => (await receipt()).id === "")
  assert.equal((await receipt()).title, "OpenCode")
  route = { name: "session", params: { sessionID: "ses_three" } }
  t.mock.timers.tick(1000)
  await waitFor(async () => (await receipt()).id === "ses_three")
  await dispose()
  await assert.rejects(readFile(file), { code: "ENOENT" })
})

async function waitFor(condition) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if (await condition()) return } catch {}
    await new Promise((resolve) => setImmediate(resolve))
  }
  assert.fail("presence did not update")
}
