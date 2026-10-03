import { test } from "node:test"
import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import plugin from "../bridge.mjs"

function reactiveForTest() {
  const effects = new Set()
  return {
    createRoot: (setup) => setup(() => effects.clear()),
    createEffect: (effect) => { effects.add(effect); effect() },
    flush: () => { for (const effect of effects) effect() },
  }
}

test("V2 presence follows router forms and model metadata without exposing credentials", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "tandem-v2-bridge-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  t.mock.timers.enable({ apis: ["setInterval"] })
  let route = { type: "session", sessionID: "ses_one" }
  let tabs = [{ sessionID: "ses_one" }, { sessionID: "ses_two" }]
  let forms = []
  const context = {
    options: { presenceDirectory: root },
    location: { directory: "/work/review" },
    client: { server: { info: async () => ({ urls: ["http://127.0.0.1:4199"], password: "must-not-leak" }) } },
    data: {
      session: {
        get: (id) => ({ title: `Native ${id}`, location: { directory: "/work/review" } }),
        status: () => "running",
        message: {
          list: () => [{ type: "user", id: "msg_one", text: "Review this" }, {
            type: "assistant", agent: "tracer", model: { id: "test", providerID: "openai", variant: "high" },
            tokens: { input: 100, output: 20, reasoning: 5, cache: { read: 10, write: 0 } },
          }],
          get: (id) => ({ type: "user", text: `Review ${id}` }),
        },
        form: { list: () => forms },
      },
      location: {
        agent: { list: () => [{ id: "tracer", color: "#FB923C" }] },
        model: { list: () => [{ id: "test", providerID: "openai", name: "Test model", limit: { context: 1000 } }] },
        provider: { list: () => [{ id: "openai", name: "OpenAI" }] },
      },
    },
    ui: {
      router: { current: () => route },
      tabs: { enabled: () => true, list: () => tabs },
    },
  }
  const reactive = reactiveForTest()
  const dispose = await plugin.setup(context, reactive)
  t.after(dispose)
  const receipt = async () => JSON.parse(await readFile(join(root, `${process.pid}.json`), "utf8"))
  assert.deepEqual({ id: (await receipt()).id, activity: (await receipt()).activity, model: (await receipt()).model,
    tokens: (await receipt()).context_tokens, limit: (await receipt()).context_limit, question: (await receipt()).last_question },
  { id: "ses_one", activity: "busy", model: "openai/test", tokens: 135, limit: 1000, question: "Review ses_one" })
  assert.deepEqual((await receipt()).tabs.map((tab) => ({ id: tab.id, question: tab.last_question })),
    [{ id: "ses_two", question: "Review ses_two" }])
  assert.equal((await receipt()).server, "http://127.0.0.1:4199")
  assert.equal((await receipt()).tab_index, 0)
  assert.equal((await receipt()).tabs[0].tab_index, 1)
  assert.ok(!JSON.stringify(await receipt()).includes("must-not-leak"))
  forms = [{ id: "form_one" }]
  reactive.flush()
  for (let attempt = 0; attempt < 100 && (await receipt()).activity !== "awaiting_answer"; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.equal((await receipt()).activity, "awaiting_answer")
  route = { type: "home" }
  reactive.flush()
  for (let attempt = 0; attempt < 100 && (await receipt()).id !== ""; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.equal((await receipt()).id, "")
  assert.deepEqual((await receipt()).tabs.map((tab) => tab.id), ["ses_one", "ses_two"])
  tabs = [{ sessionID: "ses_two" }]
  reactive.flush()
  for (let attempt = 0; attempt < 100 && (await receipt()).tabs.length !== 1; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.deepEqual((await receipt()).tabs.map((tab) => tab.id), ["ses_two"])
  route = { type: "session", sessionID: "ses_one" }
  tabs = [{ sessionID: "ses_two" }, { sessionID: "ses_one" }]
  reactive.flush()
  for (let attempt = 0; attempt < 100 && (await receipt()).tab_index !== 1; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.equal((await receipt()).tab_index, 1)
  assert.equal((await receipt()).tabs[0].tab_index, 0)
  await dispose()
  await assert.rejects(readFile(join(root, `${process.pid}.json`)), { code: "ENOENT" })
})

test("V2 initial input uses native location and text contracts once", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "tandem-v2-prompt-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const inherited = process.env.TANDEM_INITIAL_PROMPT
  process.env.TANDEM_INITIAL_PROMPT = "Literal 'text'; $(not-a-shell)"
  t.after(() => { if (inherited === undefined) delete process.env.TANDEM_INITIAL_PROMPT; else process.env.TANDEM_INITIAL_PROMPT = inherited })
  let route = { type: "home" }
  const calls = []
  const context = {
    options: { presenceDirectory: root }, location: { directory: "/work/review" },
    client: {
      server: { info: async () => ({ urls: ["http://127.0.0.1:4199"] }) },
      session: {
        create: async (input) => { calls.push(["create", input]); return { id: "ses_new" } },
        prompt: async (input) => { calls.push(["prompt", input]) },
      },
    },
    data: { session: {}, location: { provider: { list: () => [] }, model: { list: () => [] } } },
    ui: { router: { current: () => route, navigate: (next) => { route = next; calls.push(["route", next]) } }, toast: { show: () => assert.fail("unexpected error") } },
  }
  const dispose = await plugin.setup(context, reactiveForTest())
  t.after(dispose)
  assert.deepEqual(calls, [["create", { location: { directory: "/work/review" } }],
    ["route", { type: "session", sessionID: "ses_new" }],
    ["prompt", { sessionID: "ses_new", text: "Literal 'text'; $(not-a-shell)" }]])
  assert.equal(process.env.TANDEM_INITIAL_PROMPT, undefined)
})

async function tabClient(t) {
  const root = await mkdtemp(join(tmpdir(), "tandem-v2-tabs-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  let route = { type: "home" }
  let enabled = true
  const calls = []
  const context = {
    options: { presenceDirectory: root }, location: { directory: "/work/review" },
    client: {
      server: { info: async () => ({ urls: ["http://127.0.0.1:4199"] }) },
      session: { create: async (input) => { calls.push(["create", input]); return { id: "ses_new" } } },
    },
    data: { session: {}, location: { provider: { list: () => [] }, model: { list: () => [] } } },
    ui: {
      router: { current: () => route },
      tabs: {
        enabled: () => enabled,
        list: () => [],
        focus: (id) => { route = { type: "session", sessionID: id }; calls.push(["focus", id]) },
      },
    },
  }
  const dispose = await plugin.setup(context, reactiveForTest())
  t.after(dispose)
  const receipt = JSON.parse(await readFile(join(root, `${process.pid}.json`), "utf8"))
  const { server, token } = receipt.tab_control
  return {
    context, calls, dispose,
    disable: () => { enabled = false },
    request: (body = { directory: "/work/review" }, authorization = `Bearer ${token}`, path = "/tabs") => fetch(`${server}${path}`, {
      method: "POST", headers: { authorization, "Content-Type": "application/json", connection: "close" },
      body: JSON.stringify(body),
    }),
  }
}

test("authenticated tab requests create and focus one blank session in the existing client", async (t) => {
  const client = await tabClient(t)
  assert.equal((await client.request({}, "Bearer invalid")).status, 401)
  assert.equal((await client.request({ directory: "relative" })).status, 400)
  assert.deepEqual(client.calls, [])
  const response = await client.request({ directory: "/work/space ' ; $(literal)/日本語" })
  assert.equal(response.status, 200)
  assert.deepEqual(await response.json(), { id: "ses_new" })
  assert.deepEqual(client.calls, [
    ["create", { location: { directory: "/work/space ' ; $(literal)/日本語" } }], ["focus", "ses_new"],
  ])
  await client.dispose()
  await assert.rejects(client.request())
})

test("disabled tabs and uncertain creation keep the existing conversation without retrying", async (t) => {
  const client = await tabClient(t)
  client.disable()
  const disabled = await client.request()
  assert.equal(disabled.status, 409)
  assert.match((await disabled.json()).error, /Enable OpenCode session tabs/)
  assert.deepEqual(client.calls, [])
  client.context.ui.tabs.enabled = () => true
  let attempts = 0
  client.context.client.session.create = async () => { attempts++; throw new Error("connection lost") }
  const failed = await client.request()
  assert.equal(failed.status, 502)
  assert.match((await failed.json()).error, /creation may be uncertain/)
  assert.equal(attempts, 1)
  assert.deepEqual(client.calls, [])
})

test("concurrent tab requests are rejected and disposal cancels creation before focus", async (t) => {
  const client = await tabClient(t)
  let entered
  const started = new Promise((resolve) => { entered = resolve })
  let signal
  client.context.client.session.create = (_, options) => new Promise((_, reject) => {
    signal = options.signal
    signal.addEventListener("abort", () => reject(new Error("cancelled")), { once: true })
    entered()
  })
  const pending = client.request().catch(() => undefined)
  await started
  assert.equal((await client.request()).status, 409)
  await client.dispose()
  await pending
  assert.equal(signal.aborted, true)
  assert.deepEqual(client.calls, [])
})

test("navigation focuses an open background tab and rejects a closed tab", async (t) => {
  const client = await tabClient(t)
  client.context.ui.tabs.list = () => [{ sessionID: "ses_background" }]
  const focused = await client.request({ sessionID: "ses_background" }, undefined, "/tabs/focus")
  assert.equal(focused.status, 200)
  assert.deepEqual(await focused.json(), { id: "ses_background" })
  assert.deepEqual(client.calls, [["focus", "ses_background"]])
  client.context.ui.tabs.list = () => []
  const closed = await client.request({ sessionID: "ses_closed" }, undefined, "/tabs/focus")
  assert.equal(closed.status, 409)
  assert.deepEqual(client.calls, [["focus", "ses_background"]])
})

test("new-session requests reuse a verified empty idle tab in the requested workspace", async (t) => {
  const client = await tabClient(t)
  const messages = new Map([["ses_used", [{ type: "user", text: "Existing question" }]]])
  const directories = new Map([["ses_other", "/work/other"]])
  const ids = ["ses_used", "ses_other", "ses_busy", "ses_queued", "ses_question", "ses_empty"]
  const synchronized = []
  client.context.data.session = {
    get: (id) => ({ title: id, location: { directory: directories.get(id) ?? "/work/review" } }),
    status: (id) => id === "ses_busy" ? "running" : "idle",
    pending: { list: (id) => id === "ses_queued" ? [{ id: "pending_one" }] : [], sync: async () => {} },
    form: { list: (id) => id === "ses_question" ? [{ id: "form_one" }] : [], sync: async () => {} },
    message: {
      list: (id) => messages.get(id) ?? [],
      sync: async (id) => { synchronized.push(id) },
    },
  }
  client.context.ui.tabs.list = () => ids.map((sessionID) => ({ sessionID }))
  const response = await client.request()
  assert.equal(response.status, 200)
  assert.deepEqual(await response.json(), { id: "ses_empty" })
  assert.deepEqual(synchronized, ["ses_empty"])
  assert.deepEqual(client.calls, [["focus", "ses_empty"]])
  const repeated = await client.request()
  assert.equal(repeated.status, 200)
  assert.deepEqual(await repeated.json(), { id: "ses_empty" })
  assert.deepEqual(client.calls, [["focus", "ses_empty"], ["focus", "ses_empty"]])
})

test("unloaded, changing, or closed tabs must be checked before creating an empty session", async (t) => {
  for (const state of ["unloaded", "running", "queued", "question", "closed", "failed", "refused"]) {
    await t.test(state, async (t) => {
      const client = await tabClient(t)
      let messages = []
      let running = false
      let queued = []
      let forms = []
      let open = true
      client.context.data.session = {
        get: () => ({ location: { directory: "/work/review" } }),
        status: () => running ? "running" : "idle",
        pending: { list: () => queued, sync: async () => { if (state === "queued") queued = [{ id: "pending_one" }] } },
        form: { list: () => forms, sync: async () => { if (state === "question") forms = [{ id: "form_one" }] } },
        message: {
          list: () => messages,
          sync: async () => {
            if (state === "failed") throw new Error("Cannot verify messages")
            if (state === "unloaded") messages = [{ type: "user", text: "A real conversation" }]
            if (state === "running") running = true
            if (state === "closed") open = false
          },
        },
      }
      client.context.ui.tabs.list = () => open ? [{ sessionID: "ses_existing" }] : []
      if (state === "refused") client.context.ui.tabs.focus = () => false
      const response = await client.request()
      if (["failed", "refused"].includes(state)) {
        assert.equal(response.status, state === "failed" ? 502 : 409)
        assert.deepEqual(client.calls, [])
      } else {
        assert.equal(response.status, 200)
        assert.deepEqual(await response.json(), { id: "ses_new" })
        assert.deepEqual(client.calls, [["create", { location: { directory: "/work/review" } }], ["focus", "ses_new"]])
      }
    })
  }
})
