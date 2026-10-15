import { test } from "node:test"
import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { PassThrough } from "node:stream"
import { spawn } from "node:child_process"
import { instanceView, scopeEnvironment, setupToolbar } from "../toolbar.mjs"
import { openInstance, openRoute, watchState } from "../toolbar-transport.mjs"

const server = "http://127.0.0.1:4199"
const target = {
  name: "review", workspace: "/work/review", template: "website", workspace_only: false,
  summary: { status: "healthy", label: "Healthy", severity: "success", running: 3, expected: 3, busy: false },
  runtime: { stale: false }, services: [{ name: "web", url: "http://localhost:9876/review/" }],
}
const session = { server, session_id: "ses_one", instance: "review", stale: false }
const snapshot = () => ({
  connected: true, instances: new Map([["review", structuredClone(target)]]),
  sessions: new Map([[JSON.stringify([server, "ses_one"]), { ...session }]]), metadata: {},
})
const settle = () => new Promise((resolve) => setImmediate(resolve))

test("the toolbar uses exact server/session ownership and shared status projections", () => {
  const state = snapshot()
  assert.equal(instanceView(state, "http://127.0.0.1:9999", "ses_one"), undefined)
  assert.equal(instanceView(state, server, "ses_other"), undefined)
  assert.deepEqual(instanceView(state, server, "ses_one"), {
    instance: target, stale: false, label: "Healthy", severity: "success", services: "3/3 services running",
    routes: [{ title: "web", value: target.services[0].url, description: target.services[0].url }], disabled: false,
  })
  for (const staleState of [
    { ...state, connected: false },
    { ...state, metadata: { sessions: { inventory_error: "Docker unavailable" } } },
  ]) {
    assert.equal(instanceView(staleState, server, "ses_one").label, "Stale")
    assert.equal(instanceView(staleState, server, "ses_one").disabled, true)
  }
  state.instances.get("review").workspace_only = true
  assert.equal(instanceView(state, server, "ses_one").services, "")
  assert.equal(instanceView(state, server, "ses_one").label, "Workspace only")
})

test("workspace configuration supplies only the Tandem scope with explicit option precedence", () => {
  const docs = [{ type: "document", info: { mcp: { servers: { "tandem-instance": {
    type: "local", command: ["/trusted/tandem", "mcp-instance"],
    environment: { TANDEM_HOME: "/scope", TANDEM_NAMESPACE: "fixtures", TANDEM_GATEWAY_PORT: "9999", PATH: "/untrusted" },
  } } } } }]
  assert.deepEqual(scopeEnvironment(docs, { tandemHome: "/override" }, { PATH: "/bin" }), {
    PATH: "/bin", TANDEM_HOME: "/override", TANDEM_NAMESPACE: "fixtures", TANDEM_GATEWAY_PORT: "9999",
  })
})

function childForTest() {
  const child = new EventEmitter()
  child.stdout = new PassThrough()
  child.kill = () => { child.killed = true }
  child.unref = () => {}
  return child
}

test("state publications update the footer, disconnect as stale, and reconnect from a fresh snapshot", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] })
  const children = [], states = [], calls = []
  const stop = watchState({}, (value) => states.push(value), (binary, args) => {
    calls.push(args)
    const child = childForTest()
    children.push(child)
    return child
  })
  t.after(stop)
  const frame = (index, type, data) => children[index].stdout.write(`${JSON.stringify({ version: 1, type, data })}\n`)
  frame(0, "snapshot", { instances: { instances: [target] }, sessions: { sessions: [session] } })
  assert.equal(instanceView(states.at(-1), server, "ses_one").label, "Healthy")
  frame(0, "instance.changed", { ...target, summary: { ...target.summary, label: "Unhealthy" } })
  assert.equal(instanceView(states.at(-1), server, "ses_one").label, "Unhealthy")
  frame(0, "future.event", { ignored: true })
  children[0].emit("close", 1)
  assert.equal(instanceView(states.at(-1), server, "ses_one").label, "Stale")
  assert.deepEqual(calls[1], ["serve-state"])
  t.mock.timers.tick(1000)
  assert.deepEqual(calls[2], ["watch", "--json"])
  frame(2, "snapshot", { instances: { instances: [target] }, sessions: { sessions: [session] } })
  assert.equal(instanceView(states.at(-1), server, "ses_one").label, "Healthy")
  frame(2, "session.removed", session)
  assert.equal(instanceView(states.at(-1), server, "ses_one"), undefined)
  stop()
  assert.equal(children[2].killed, true)
})

function mcpFixture(identity = target, loseAction = false) {
  return (_binary, args, options) => {
    assert.deepEqual(args, ["mcp-instance"])
    assert.equal(options.cwd, target.workspace)
    return spawn(process.execPath, ["--input-type=module", "-e", `
      import { createInterface } from 'node:readline';
      for await (const line of createInterface({ input: process.stdin })) {
        const request = JSON.parse(line);
        if (!request.id) continue;
        let result = {};
        if (request.method === 'initialize') result = { protocolVersion: '2024-11-05', capabilities: {}, serverInfo: { name: 'fixture', version: '1' } };
        if (request.method === 'tools/call') {
          if (request.params.name !== 'get_instructions' && ${loseAction}) process.exit(0);
          result = { structuredContent: request.params.name === 'get_instructions' ? ${JSON.stringify({ ...identity, instance: identity.name })} : { action: request.params.name }, content: [] };
        }
        process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }) + '\\n');
      }
    `], { ...options, cwd: process.cwd() })
  }
}

test("lifecycle transport binds identity before sending explicit instance-local actions", async (t) => {
  const control = await openInstance(target, {}, mcpFixture())
  t.after(control.close)
  assert.deepEqual(await control.action("start"), { action: "start_self" })
  assert.deepEqual(await control.action("stop"), { action: "stop_self" })
  assert.deepEqual(await control.action("purge"), { action: "purge" })
  await assert.rejects(openInstance(target, {}, mcpFixture({ ...target, template: "other" })), /identity changed/)
})

test("lost action replies surface uncertainty without resending", async (t) => {
  const control = await openInstance(target, {}, mcpFixture(target, true))
  t.after(control.close)
  await assert.rejects(control.action("purge"), /operation may still be running/)
})

test("route opening passes one literal HTTP URL to the browser and rejects credentials and executable schemes", async () => {
  const opened = []
  const launch = (command, args) => {
    opened.push({ command, args })
    const child = new EventEmitter()
    queueMicrotask(() => child.emit("exit", 0))
    return child
  }
  for (const [platform, command] of [["linux", "xdg-open"], ["darwin", "open"]]) {
    opened.length = 0
    await openRoute("http://localhost:9876/review/?q=$(literal)", launch, async () => undefined, platform)
    assert.deepEqual(opened, [{ command, args: ["http://localhost:9876/review/?q=$(literal)"] }])
  }
  await assert.rejects(openRoute("https://user:secret@example.com/", launch), /without credentials/)
  await assert.rejects(openRoute("file:///etc/passwd", launch), /HTTP/)
})

test("Linux routes use the scheme's desktop handler with GTK activation and fall back only before launching", async () => {
  const opened = [], schemes = []
  let failure
  const launch = (command, args) => {
    opened.push({ command, args })
    const child = new EventEmitter()
    queueMicrotask(() => {
      if (command === "gtk-launch" && failure === "missing") child.emit("error", Object.assign(new Error("missing"), { code: "ENOENT" }))
      else child.emit("exit", command === "gtk-launch" && failure === "uncertain" ? 1 : 0)
    })
    return child
  }
  const browser = async (scheme) => { schemes.push(scheme); return "browser.desktop" }
  const url = "https://localhost:9876/review/?q=$(literal)"
  await openRoute(url, launch, browser, "linux")
  assert.deepEqual(schemes, ["https:"])
  assert.deepEqual(opened, [{ command: "gtk-launch", args: ["browser.desktop", url] }])
  opened.length = 0
  failure = "missing"
  await openRoute(url, launch, browser, "linux")
  assert.deepEqual(opened, [
    { command: "gtk-launch", args: ["browser.desktop", url] },
    { command: "xdg-open", args: [url] },
  ])
  opened.length = 0
  failure = "uncertain"
  await assert.rejects(openRoute(url, launch, browser, "linux"), /Could not open/)
  assert.deepEqual(opened, [{ command: "gtk-launch", args: ["browser.desktop", url] }])
})

test("native Windows routes pass URLs as data to the default browser without evaluating them as PowerShell", async () => {
  const opened = []
  let exitCode = 0
  const launch = (command, args, options) => {
    opened.push({ command, args, options })
    const child = new EventEmitter()
    queueMicrotask(() => child.emit("exit", exitCode))
    return child
  }
  const browser = () => { throw new Error("Windows must not query Linux desktop handlers") }
  const url = "https://example.test/?q=';Start-Process calc;&x=$(whoami)#fragment"
  await openRoute(url, launch, browser, "win32")
  assert.equal(opened.length, 1)
  assert.equal(opened[0].command, "powershell.exe")
  assert.deepEqual(opened[0].args, ["-NoProfile", "-NonInteractive", "-Command",
    "Start-Process -FilePath $env:TANDEM_ROUTE_URL -Verb Open -ErrorAction Stop"])
  assert.equal(opened[0].options.env.TANDEM_ROUTE_URL, new URL(url).href)
  assert.equal(opened[0].options.windowsHide, true)
  assert.equal(opened[0].options.stdio, "ignore")
  opened.length = 0
  exitCode = 1
  await assert.rejects(openRoute(url, launch, browser, "win32"), /Could not open/)
  assert.equal(opened.length, 1)
  opened.length = 0
  for (const invalid of ["javascript:alert(1)", "file:///C:/Windows/notepad.exe", "https://user:secret@example.test/"]) {
    await assert.rejects(openRoute(invalid, launch, browser, "win32"), /HTTP\(S\) URLs without credentials/)
  }
  assert.equal(opened.length, 0)
})

async function toolbarFixture(t, dependencies = {}) {
  let route = { type: "session", sessionID: "ses_one" }, changed, commands, select, confirm
  const effects = [], slots = [], actions = [], opened = [], toasts = [], configurations = [], watchers = []
  const reactive = {
    createRoot: (setup) => setup(() => {}),
    createSignal: (value) => [() => value, (next) => { value = next }],
    createEffect: (effect) => { effects.push(effect); effect() },
    Show: (props) => props,
  }
  const elements = {
    createComponent: (component, props) => component(props), createElement: (type) => ({ type, children: [] }),
    spread: (node, props) => { node.props = props }, insert: (parent, node) => parent.children.push(node),
  }
  const context = {
    options: {},
    client: { config: { get: async () => [{ type: "document", info: { mcp: { servers: {
      "tandem-instance": { type: "local", command: ["tandem", "mcp-instance"], environment: { TANDEM_HOME: "/fixture/home" } },
    } } } }] } },
    data: { session: { get: () => ({ location: { directory: target.workspace } }) } },
    theme: {
      text: { muted: "grey", feedback: { success: { base: "green" }, warning: { base: "amber" }, error: { base: "red" } } },
    },
    keymap: { layer: (factory) => { commands = factory().commands } },
    ui: {
      router: { current: () => route }, toast: { show: (value) => toasts.push(value) },
      slot: (slot) => { slots.push(slot); return () => { slot.removed = true } },
      dialog: { select: (options) => select(options), confirm: (options) => confirm(options) },
    },
  }
  const dispose = await setupToolbar(context, server, {
    reactive, elements,
    watchState: (options, receive) => {
      configurations.push(options)
      changed = receive
      const watcher = { stopped: false }
      watchers.push(watcher)
      return () => { watcher.stopped = true }
    },
    openInstance: async () => ({ close() {}, action: async (action) => { actions.push(action) } }),
    openRoute: async (url) => { opened.push(url) },
    ...dependencies,
  })
  t.after(dispose)
  slots.find((slot) => slot.append === "app").render()
  const footer = slots.find((slot) => slot.before === "prompt.footer.status").render()
  await settle()
  return {
    actions, opened, toasts, configurations, watchers, slots, footer, dispose,
    publish: (state) => changed(state),
    choose: (callback) => { select = callback }, confirm: (callback) => { confirm = callback },
    run: (name) => commands.find((command) => command.id === `tandem.instance.${name}`).run(),
    command: (name) => commands.find((command) => command.id === `tandem.instance.${name}`),
    navigate(next) { route = next; for (const effect of effects) effect() },
  }
}

test("instance menus require explicit selection and Purge confirmation", async (t) => {
  const ui = await toolbarFixture(t)
  assert.equal(ui.configurations[0].env.TANDEM_HOME, "/fixture/home")
  assert.equal(ui.footer.when, undefined)
  ui.publish(snapshot())
  ui.choose(() => "stop")
  await ui.run("actions")
  assert.deepEqual(ui.actions, ["stop"])
  ui.choose(() => "purge")
  ui.confirm(() => false)
  await ui.run("actions")
  assert.deepEqual(ui.actions, ["stop"])
  ui.confirm((dialog) => { assert.match(dialog.message, /cannot be undone/); return true })
  await ui.run("actions")
  assert.deepEqual(ui.actions, ["stop", "purge"])
  ui.choose((dialog) => dialog.options[0].value)
  await ui.run("routes")
  assert.deepEqual(ui.opened, [target.services[0].url])
  ui.navigate({ type: "session", sessionID: "ses_outside" })
  assert.equal(ui.footer.when, undefined)
  assert.equal(ui.watchers[0].stopped, true)
})

test("workspace-only instances hide service controls and session changes cancel menu actions", async (t) => {
  const ui = await toolbarFixture(t)
  const state = snapshot()
  state.instances.get("review").workspace_only = true
  ui.publish(state)
  ui.choose((dialog) => {
    assert.deepEqual(dialog.options.map((option) => option.value), ["purge"])
    ui.navigate({ type: "home" })
    return "purge"
  })
  ui.confirm(() => true)
  await ui.run("actions")
  assert.deepEqual(ui.actions, [])
  assert.match(ui.toasts.at(-1).message, /selected instance changed/)
  ui.dispose()
  assert.ok(ui.slots.every((slot) => slot.removed))
})

test("service menus disable redundant actions while preserving recovery and paused-instance controls", async (t) => {
  const ui = await toolbarFixture(t)
  for (const [status, running, startDisabled, stopDisabled] of [
    ["healthy", 3, true, false],
    ["unhealthy", 3, true, false],
    ["stopped", 0, false, true],
    ["not_started", 0, false, true],
    ["completed", 0, false, true],
    ["degraded", 1, false, false],
    ["paused", 0, false, false],
    ["restarting", 0, false, false],
  ]) {
    const state = snapshot()
    Object.assign(state.instances.get("review").summary, { status, running })
    ui.publish(state)
    ui.choose((dialog) => {
      assert.deepEqual(dialog.options.map(({ value, disabled }) => [value, Boolean(disabled)]), [
        ["start", startDisabled], ["stop", stopDisabled], ["purge", false],
      ], status)
      return startDisabled ? "start" : stopDisabled ? "stop" : undefined
    })
    await ui.run("actions")
    assert.deepEqual(ui.actions, [], status)
  }
})

test("an action becoming unavailable while its menu is open is rejected before execution", async (t) => {
  const ui = await toolbarFixture(t)
  const state = snapshot()
  Object.assign(state.instances.get("review").summary, { status: "stopped", running: 0 })
  ui.publish(state)
  ui.choose((dialog) => {
    assert.equal(dialog.options.find(({ value }) => value === "start").disabled, false)
    ui.publish(snapshot())
    return "start"
  })
  await ui.run("actions")
  assert.deepEqual(ui.actions, [])
  assert.match(ui.toasts.at(-1).message, /selected action is unavailable/)
})

test("the footer dot follows instance health and hides outside its session", async (t) => {
  const ui = await toolbarFixture(t)
  assert.equal(ui.footer.when, undefined)
  const state = snapshot()
  ui.publish(state)
  const dot = ui.footer.children()
  assert.equal(dot.props.content, "● ")
  assert.equal(dot.props.width, 2)
  for (const [severity, color] of [["success", "green"], ["error", "red"], ["muted", "grey"]]) {
    state.instances.get("review").summary.severity = severity
    ui.publish(state)
    assert.equal(dot.props.fg, color)
  }
  ui.publish({ ...state, connected: false })
  assert.equal(dot.props.fg, "amber")
  assert.equal(ui.command("actions").enabled(), false)
  ui.navigate({ type: "session", sessionID: "ses_outside" })
  assert.equal(ui.footer.when, undefined)
})

test("instance shortcuts guard against overlapping dialogs and operations", async (t) => {
  let select, finish
  const ui = await toolbarFixture(t, {
    openInstance: async () => ({ close() {}, action: () => new Promise((resolve) => { finish = resolve }) }),
  })
  ui.publish(snapshot())
  assert.equal(ui.command("actions").bind, "<leader>i")
  assert.equal(ui.command("routes").bind, "<leader>r")
  assert.equal(ui.command("actions").enabled(), true)
  assert.equal(ui.command("routes").enabled(), true)
  ui.choose(() => new Promise((resolve) => { select = resolve }))
  const action = ui.run("actions")
  await settle()
  assert.equal(ui.command("actions").enabled(), false)
  assert.equal(ui.command("routes").enabled(), false)
  select("stop")
  await settle()
  assert.equal(ui.command("actions").enabled(), false)
  assert.equal(ui.command("routes").enabled(), false)
  finish()
  await action
  assert.equal(ui.command("actions").enabled(), true)
  assert.equal(ui.command("routes").enabled(), true)
})
