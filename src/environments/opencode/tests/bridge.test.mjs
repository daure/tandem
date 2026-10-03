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
  let questions = []
  const api = {
    route: { get current() { return route } },
    state: {
      ready: true,
      path: { directory: "/work/review" },
      session: {
        get: (id) => ({ title: `Title ${id}`, directory: "/work/review" }),
        messages: () => [{
          role: "assistant",
          agent: "tracer",
          providerID: "openai",
          modelID: "gpt-5.6-sol",
          variant: "high",
          tokens: { input: 150_000, output: 1_000, reasoning: 0, cache: { read: 0, write: 0 } },
        }],
        status: () => ({ type: activity }),
        question: () => questions,
      },
      config: { agent: { tracer: { color: "#FB923C" } } },
      provider: [{
        id: "openai",
        name: "OpenAI",
        models: { "gpt-5.6-sol": { name: "GPT-5.6 Sol", limit: { context: 400_000 } } },
      }],
    },
    lifecycle: { onDispose: (fn) => { dispose = fn } },
  }
  const file = join(root, `${process.pid}.json`)
  const receipt = async () => JSON.parse(await readFile(file, "utf8"))
  await plugin.tui(api, { presenceDirectory: root })
  assert.equal((await receipt()).id, "ses_one")
  assert.equal((await receipt()).activity, "busy")
  assert.deepEqual(
    {
      agent: (await receipt()).agent,
      agentColor: (await receipt()).agent_color,
      model: (await receipt()).model,
      modelName: (await receipt()).model_name,
      providerName: (await receipt()).provider_name,
      variant: (await receipt()).variant,
    },
    {
      agent: "tracer",
      agentColor: "#FB923C",
      model: "openai/gpt-5.6-sol",
      modelName: "GPT-5.6 Sol",
      providerName: "OpenAI",
      variant: "high",
    },
  )
  questions = [{ id: "que_one", sessionID: "ses_one" }]
  t.mock.timers.tick(1000)
  await waitFor(async () => (await receipt()).activity === "awaiting_answer")
  questions = []
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
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.fail("presence did not update")
}

async function promptClient(t, prompt, ready = true) {
  const root = await mkdtemp(join(tmpdir(), "tandem-prompt-"))
  const inherited = process.env.TANDEM_INITIAL_PROMPT
  if (prompt === undefined) delete process.env.TANDEM_INITIAL_PROMPT
  else process.env.TANDEM_INITIAL_PROMPT = prompt
  t.mock.timers.enable({ apis: ["setInterval"] })
  let dispose
  const calls = []
  const api = {
    route: {
      current: { name: "home" },
      navigate(name, params) {
        calls.push(["navigate", name, params])
        this.current = { name, params }
      },
    },
    state: {
      ready,
      path: { directory: "/work/review" },
      session: {
        get: () => ({ title: "New session", directory: "/work/review" }),
        messages: () => [],
        status: () => ({ type: "idle" }),
        question: () => [],
      },
    },
    client: { session: {
      async create(input, options) {
        calls.push(["create", input, options])
        return { data: { id: "ses_new" } }
      },
      async promptAsync(input, options) { calls.push(["prompt", input, options]) },
      async prompt(input, options) { calls.push(["context", input, options]) },
    } },
    ui: { toast: (input) => calls.push(["toast", input]) },
    lifecycle: { onDispose: (fn) => { dispose = fn } },
  }
  t.after(async () => {
    await dispose?.()
    await rm(root, { recursive: true, force: true })
    if (inherited === undefined) delete process.env.TANDEM_INITIAL_PROMPT
    else process.env.TANDEM_INITIAL_PROMPT = inherited
  })
  return {
    api, calls,
    start: () => plugin.tui(api, { presenceDirectory: root }),
    dispose: () => dispose(),
    receipt: async () => JSON.parse(await readFile(join(root, `${process.pid}.json`), "utf8")),
  }
}

test("initial prompts wait for readiness, navigate locally, and submit literal text once", async (t) => {
  const text = "Explain 'this'; $(touch injected)\nsecond line"
  const client = await promptClient(t, text, false)
  await client.start()
  assert.equal(process.env.TANDEM_INITIAL_PROMPT, undefined)
  assert.deepEqual(client.calls, [])
  client.api.state.ready = true
  t.mock.timers.tick(1000)
  await waitFor(() => client.calls.length === 3)
  const [create, navigate, prompt] = client.calls
  assert.deepEqual(create.slice(0, 2), ["create", { directory: "/work/review" }])
  assert.equal(create[2].throwOnError, true)
  assert.ok(create[2].signal instanceof AbortSignal)
  assert.deepEqual(navigate, ["navigate", "session", { sessionID: "ses_new" }])
  assert.deepEqual(prompt.slice(0, 2), ["prompt", {
    directory: "/work/review", sessionID: "ses_new", parts: [{ type: "text", text }],
  }])
  client.api.state.session.messages = () => [{ id: "msg_initial", role: "user" }]
  client.api.state.part = () => [{ type: "text", text }]
  t.mock.timers.tick(3000)
  await waitFor(async () => (await client.receipt()).id === "ses_new")
  assert.equal((await client.receipt()).server, "")
  assert.equal((await client.receipt()).last_question, "Explain 'this'; $(touch injected) second line")
  client.api.state.session.messages = () => [
    { id: "msg_initial", role: "user" },
    { id: "msg_followup", role: "user" },
  ]
  client.api.state.part = (id) => id === "msg_followup"
    ? [{ type: "text", text: "Now review the tests" }, { type: "file", filename: "tests.rs" }]
    : [{ type: "text", text }]
  t.mock.timers.tick(1000)
  await waitFor(async () => (await client.receipt()).last_question === "Now review the tests")
  assert.equal(client.calls.length, 3)
})

test("absent and blank prompts leave the client on its home route", async (t) => {
  for (const prompt of [undefined, "", " \n\t"]) {
    await t.test(JSON.stringify(prompt) ?? "absent", async (t) => {
      const client = await promptClient(t, prompt)
      await client.start()
      assert.deepEqual(client.calls, [])
      assert.equal(process.env.TANDEM_INITIAL_PROMPT, undefined)
      assert.equal((await client.receipt()).id, "")
    })
  }
})

test("V1 instructions are persisted without a model reply before initial task input", async (t) => {
  for (const text of [undefined, "Explore the code"]) {
    await t.test(String(text), async (t) => {
      const client = await promptClient(t, text)
      process.env.TANDEM_SESSION_INSTRUCTIONS = "Services won't start automatically"
      t.after(() => { delete process.env.TANDEM_SESSION_INSTRUCTIONS })
      await client.start()
      assert.deepEqual(client.calls.map(([kind]) => kind), text ? ["create", "context", "navigate", "prompt"] : ["create", "context", "navigate"])
      assert.deepEqual(client.calls[1][1], {
        directory: "/work/review", sessionID: "ses_new", noReply: true,
        parts: [{ type: "text", text: "Services won't start automatically", synthetic: true }],
      })
      assert.equal(process.env.TANDEM_SESSION_INSTRUCTIONS, undefined)
    })
  }
})

test("failed creation or uncertain submission reports one error without automatic retries", async (t) => {
  for (const method of ["create", "promptAsync"]) {
    await t.test(method, async (t) => {
      const client = await promptClient(t, "Start work")
      let attempts = 0
      client.api.client.session[method] = async () => {
        attempts++
        throw new Error("connection lost")
      }
      await client.start()
      const toast = client.calls.find(([kind]) => kind === "toast")
      assert.equal(toast[1].variant, "error")
      assert.match(toast[1].message, /delivery may be uncertain/)
      t.mock.timers.tick(3000)
      await client.dispose()
      assert.equal(attempts, 1)
      assert.equal(client.calls.filter(([kind]) => kind === "toast").length, 1)
    })
  }
})

test("client disposal cancels pending session creation before navigation or submission", async (t) => {
  const client = await promptClient(t, "Start work", false)
  let signal
  client.api.client.session.create = (_, options) => new Promise((_, reject) => {
    signal = options.signal
    signal.addEventListener("abort", () => reject(new Error("cancelled")), { once: true })
  })
  await client.start()
  client.api.state.ready = true
  t.mock.timers.tick(1000)
  await waitFor(() => signal)
  await client.dispose()
  assert.equal(signal.aborted, true)
  assert.deepEqual(client.calls, [])
})

test("initial prompt delivery does not replace a conversation chosen during startup", async (t) => {
  const client = await promptClient(t, "Start work")
  client.api.client.session.create = async () => {
    client.api.route.current = { name: "session", params: { sessionID: "ses_chosen" } }
    return { data: { id: "ses_new" } }
  }
  await client.start()
  assert.deepEqual(client.api.route.current, { name: "session", params: { sessionID: "ses_chosen" } })
  assert.deepEqual(client.calls.map(([kind]) => kind), ["toast"])
})
