import { test } from "node:test"
import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import plugin from "../bridge.mjs"

async function client(t, { running = false, forms = [], permissions = [] } = {}) {
  const root = await mkdtemp(join(tmpdir(), "tandem-existing-input-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  let route = { type: "home" }
  const calls = []
  const saved = { id: "ses_existing", agent: "build", model: { providerID: "saved", id: "model", variant: "medium" }, location: { directory: "/work/review" } }
  const context = {
    options: { presenceDirectory: root }, location: { ...saved.location },
    client: {
      server: { info: async () => ({ urls: ["http://127.0.0.1:4199"] }) },
      agent: { get: async ({ agentID }) => {
        if (agentID !== "tracer") throw new Error("Unknown agent")
        return { id: agentID }
      } },
      session: {
        get: async ({ sessionID }) => { assert.equal(sessionID, saved.id); return structuredClone(saved) },
        active: async () => running ? { [saved.id]: {} } : {},
        create: () => assert.fail("must preserve the existing conversation"),
        switchAgent: async ({ sessionID, agent }) => { assert.equal(sessionID, saved.id); calls.push(["agent", agent]); saved.agent = agent },
        switchModel: async ({ sessionID, model }) => { assert.equal(sessionID, saved.id); calls.push(["model", model]); saved.model = model },
        interrupt: async (input) => { calls.push(["interrupt", input]); running = false; forms = []; permissions = []; return { interrupted: true } },
        prompt: async (input) => { calls.push(["prompt", input]); return { sessionID: saved.id, delivery: input.delivery } },
        instructions: { entry: { put: async (input) => calls.push(["instructions", input]) } },
      },
    },
    data: {
      session: {
        form: { sync: async () => {}, list: () => forms },
        permission: { sync: async () => {}, list: () => permissions },
      },
    },
    ui: {
      model: { current: () => ({ providerID: "different", modelID: "client", variant: "low" }) },
      router: { current: () => route, navigate: (next) => { route = next; calls.push(["route", next]) } },
      tabs: { enabled: () => true, list: () => [], focus: (sessionID) => { route = { type: "session", sessionID }; calls.push(["focus", sessionID]) } },
    },
  }
  const dispose = await plugin.setup(context, { createRoot: (setup) => setup(() => {}), createEffect: (effect) => effect() })
  t.after(dispose)
  const { tab_control: { server, token } } = JSON.parse(await readFile(join(root, `${process.pid}.json`), "utf8"))
  return {
    context, saved, calls,
    request: (input = {}) => fetch(`${server}/sessions/prompt`, {
      method: "POST", headers: { authorization: `Bearer ${token}`, "Content-Type": "application/json", connection: "close" },
      body: JSON.stringify({ directory: "/work/review", sessionID: saved.id, initial_prompt: "Literal 'text'; $(not-a-shell)\n日本語", ...input }),
    }),
  }
}

test("existing prompts preserve session settings and queue literal input by default", async (t) => {
  for (const running of [false, true]) {
    await t.test(String(running), async (t) => {
      const instance = await client(t, { running })
      const before = structuredClone(instance.saved)
      const response = await instance.request({ instructions: "Services are stopped" })
      assert.equal(response.status, 200)
      assert.deepEqual(await response.json(), { session_id: "ses_existing", server: "http://127.0.0.1:4199", prompt_outcome: "queued", error: null })
      assert.deepEqual(instance.saved, before)
      assert.deepEqual(instance.calls, [
        ["focus", "ses_existing"],
        ["instructions", { sessionID: "ses_existing", key: "tandem.services", value: "Services are stopped" }],
        ["prompt", { sessionID: "ses_existing", text: "Literal 'text'; $(not-a-shell)\n日本語", delivery: "queue", resume: true }],
      ])
    })
  }
})

test("existing model, variant and agent overrides persist before queued input", async (t) => {
  for (const selection of [{ model: "openai/test" }, { variant: "high" }, { model: "openai/test#high", variant: "high", agent: "tracer" }, { agent: "tracer" }]) {
    await t.test(JSON.stringify(selection), async (t) => {
      const instance = await client(t, { running: true })
      const response = await instance.request(selection)
      assert.equal((await response.json()).prompt_outcome, "queued")
      const expectedModel = selection.model
        ? { providerID: "openai", id: "test", ...(selection.variant || selection.model.includes("#") ? { variant: selection.variant ?? "high" } : {}) }
        : { providerID: "saved", id: "model", variant: selection.variant ?? "medium" }
      assert.deepEqual(instance.saved.model, expectedModel)
      assert.equal(instance.saved.agent, selection.agent ?? "build")
      assert.equal(instance.calls.at(-1)[0], "prompt")
      assert.ok(!instance.calls.some(([action]) => action === "interrupt"))
      instance.calls.length = 0
      await instance.request()
      assert.deepEqual(instance.saved.model, expectedModel)
      assert.deepEqual(instance.calls.map(([action]) => action), ["focus", "prompt"])
    })
  }
})

test("abort refuses active sessions, forms and permission requests before changing settings", async (t) => {
  for (const activity of [{ running: true }, { forms: [{}] }, { permissions: [{}] }]) {
    await t.test(JSON.stringify(activity), async (t) => {
      const instance = await client(t, activity)
      const response = await instance.request({ when_busy: "abort", agent: "tracer", model: "openai/test" })
      const outcome = await response.json()
      assert.equal(outcome.prompt_outcome, "not_submitted")
      assert.match(outcome.error, /busy or awaiting an answer/)
      assert.deepEqual(instance.calls, [])
    })
  }
})

test("interrupt stops active work before applying overrides and sends steering input", async (t) => {
  const instance = await client(t, { running: true })
  const response = await instance.request({ when_busy: "interrupt", agent: "tracer" })
  assert.equal((await response.json()).prompt_outcome, "submitted")
  assert.deepEqual(instance.calls.map(([action]) => action), ["interrupt", "focus", "agent", "prompt"])
  assert.deepEqual(instance.calls[0][1], { sessionID: "ses_existing", resume: false })
  assert.equal(instance.calls.at(-1)[1].delivery, "steer")
})

test("existing prompt failures retain identity without resending or creating conversations", async (t) => {
  for (const stage of ["identity", "agent", "switchModel", "interrupt", "prompt"]) {
    await t.test(stage, async (t) => {
      const instance = await client(t, { running: true })
      let attempts = 0
      const fail = async () => { attempts++; throw new Error("connection lost") }
      if (stage === "identity") instance.saved.location.directory = "/another/workspace"
      else if (stage !== "agent") instance.context.client.session[stage] = fail
      const response = await instance.request({ when_busy: stage === "interrupt" ? "interrupt" : "queue", ...(stage === "agent" ? { agent: "missing" } : {}), ...(stage === "switchModel" ? { model: "openai/test" } : {}) })
      const outcome = await response.json()
      assert.equal(outcome.session_id, "ses_existing")
      assert.equal(outcome.prompt_outcome, stage === "prompt" ? "uncertain" : "not_submitted")
      assert.equal(attempts, ["identity", "agent"].includes(stage) ? 0 : 1)
      if (["identity", "agent"].includes(stage)) assert.deepEqual(instance.calls, [])
    })
  }
})
