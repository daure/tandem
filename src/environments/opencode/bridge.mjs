import { mkdir, rename, rm, writeFile } from "node:fs/promises"
import { homedir } from "node:os"
import { isAbsolute, join } from "node:path"
import { createServer } from "node:http"
import { randomBytes } from "node:crypto"

// Runs in each TUI, not in the shared server: route and terminal identity are client-local.
const plugin = {
  id: "tandem.presence",
  async setup(context, reactive) {
    const { createRoot, createEffect } = reactive ?? await import("solid-js")
    let dispose
    const info = await context.client.server.info({ signal: AbortSignal.timeout(10_000) })
    const server = info.urls.find((value) => /^http:\/\/(127\.0\.0\.1|localhost|\[::1\]):\d+\/?$/.test(value))?.replace(/\/$/, "")
    if (!server) throw new Error("OpenCode did not provide a local server identity")
    const location = () => context.location ?? context.data.location.default()
    const api = {
      server,
      tabs: context.ui.tabs,
      route: {
        get current() {
          const route = context.ui.router.current()
          return { name: route.type, params: route.type === "session" ? { sessionID: route.sessionID } : {} }
        },
        navigate(name, params) { context.ui.router.navigate({ type: name, ...params }) },
      },
      state: {
        get ready() { return Boolean(location()?.directory) },
        get path() { return location() },
        session: {
          get(id) {
            const session = context.data.session.get(id)
            return session ? { ...session, directory: session.location.directory } : undefined
          },
          messages(id) { return context.data.session.message.list(id).map(legacyMessage) },
          sync(id) { return context.data.session.sync(id) },
          syncMessages(id) { return context.data.session.message.sync(id) },
          syncPending(id) { return context.data.session.pending.sync(id) },
          syncQuestions(id) { return context.data.session.form.sync(id, context.data.session.get(id)?.location ?? location()) },
          pending(id) { return context.data.session.pending.list(id) },
          status(id) { return { type: context.data.session.status(id) === "running" ? "busy" : "idle" } },
          question(id) { return context.data.session.form.list(id, context.data.session.get(id)?.location ?? location()) ?? [] },
        },
        part(id, sessionID) {
          const route = context.ui.router.current()
          const message = sessionID || route.type === "session"
            ? context.data.session.message.get(sessionID ?? route.sessionID, id) : undefined
          return message?.type === "user" ? [{ type: "text", text: message.text }] : []
        },
        get config() {
          return { agent: Object.fromEntries((context.data.location.agent.list(location()) ?? []).map((agent) => [agent.id, agent])) }
        },
        get provider() {
          const models = context.data.location.model.list(location()) ?? []
          return (context.data.location.provider.list(location()) ?? []).map((provider) => ({
            ...provider,
            models: Object.fromEntries(models.filter((model) => model.providerID === provider.id).map((model) => [model.id, model])),
          }))
        },
      },
      client: { session: {
        async create({ directory, model }, options) {
          return { data: await context.client.session.create({ location: { directory }, ...(model ? { model } : {}) }, options) }
        },
        async attachInstructions(sessionID, instructions, options) {
          const entry = context.client.session?.instructions?.entry
          if (!entry?.put) throw new Error("This OpenCode server does not support session instruction entries")
          await entry.put({ sessionID, key: "tandem.services", value: instructions }, options)
        },
        promptAsync({ sessionID, parts }, options) {
          return context.client.session.prompt({ sessionID, text: parts.map((part) => part.text).join("\n") }, options)
        },
      } },
      ui: { toast: (options) => context.ui.toast.show(options) },
      lifecycle: {
        onDispose(fn) { dispose = fn },
        watch(read, changed) {
          return createRoot((stop) => {
            createEffect(() => changed(read()))
            return stop
          })
        },
        listen: context.data.listen ? (changed) => context.data.listen(changed) : undefined,
      },
    }
    await plugin.tui(api, context.options)
    return () => dispose?.()
  },
  async tui(api, options = {}) {
    let initialPrompt = process.env.TANDEM_INITIAL_PROMPT
    let initialInstructions = process.env.TANDEM_SESSION_INSTRUCTIONS || undefined
    let initialModel = process.env.TANDEM_SESSION_MODEL || undefined
    delete process.env.TANDEM_INITIAL_PROMPT
    delete process.env.TANDEM_SESSION_INSTRUCTIONS
    delete process.env.TANDEM_SESSION_MODEL
    const root = options.presenceDirectory ?? join(
      process.env.XDG_STATE_HOME ?? join(homedir(), ".local/state"), "tandem/opencode",
    )
    const file = join(root, `${process.pid}.json`)
    const temporary = `${file}.tmp`
    const lastQuestions = new Map()
    let stopped = false
    let pending = Promise.resolve()
    const initialRequest = new AbortController()
    const tabControl = api.tabs ? await serveTabs(api, initialRequest.signal) : undefined

    const initializeConversation = async () => {
      if (stopped || !api.state.ready || (initialPrompt === undefined && initialInstructions === undefined)) return
      const text = initialPrompt
      const instructions = initialInstructions
      const modelName = initialModel
      // Consume before awaiting: an uncertain HTTP result must never trigger a second submission.
      initialPrompt = undefined
      initialInstructions = undefined
      initialModel = undefined
      if (!text?.trim() && !instructions) return
      try {
        if (api.route.current.name !== "home") throw new Error("Client already has a conversation")
        const directory = api.state.path.directory
        const options = {
          throwOnError: true,
          signal: AbortSignal.any([initialRequest.signal, AbortSignal.timeout(15_000)]),
        }
        const [modelID, variant] = modelName?.split("#") ?? []
        const [providerID, ...modelPath] = modelID?.split("/") ?? []
        const model = modelName ? { providerID, id: modelPath.join("/"), ...(variant ? { variant } : {}) } : undefined
        const result = await api.client.session.create({ directory, ...(model ? { model } : {}) }, options)
        if (stopped) return
        if (!result.data?.id) throw new Error("Session creation returned no session")
        if (api.route.current.name !== "home") throw new Error("Client changed conversations")
        await applyInstructions(api, result.data.id, directory, instructions, options)
        if (stopped) return
        if (api.route.current.name !== "home") throw new Error("Client changed conversations")
        api.route.navigate("session", { sessionID: result.data.id })
        if (text?.trim()) await api.client.session.promptAsync({
          directory,
          sessionID: result.data.id,
          parts: [{ type: "text", text }],
          ...(model ? { model: { providerID: model.providerID, modelID: model.id }, variant } : {}),
        }, options)
      } catch {
        if (!stopped) api.ui.toast({
          variant: "error",
          title: "Tandem session setup",
          message: "Could not configure session instructions or submit the initial prompt. Check this conversation before retrying; delivery may be uncertain.",
        })
      }
    }

    const recordFor = (sessionID) => {
      const session = sessionID ? api.state.session.get(sessionID) : undefined
      if (sessionID && !session) return
      const messages = sessionID ? api.state.session.messages(sessionID) : []
      const question = messages.findLast((message) => message.role === "user")
      const questionText = question
        ? api.state.part(question.id, sessionID)
          .filter((part) => part.type === "text")
          .map((part) => part.text)
          .join(" ")
          .replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/gu, "")
          .replace(/\s+/gu, " ").trim()
        : ""
      if (questionText) lastQuestions.set(sessionID, Array.from(questionText).slice(0, 4096).join(""))
      const reply = messages.findLast((message) => message.role === "assistant")
      const last = messages.findLast((message) => message.role === "assistant" && message.tokens.output > 0)
      const provider = reply
        ? api.state.provider.find((provider) => provider.id === reply.providerID)
        : undefined
      const model = reply ? provider?.models[reply.modelID] : undefined
      const contextTokens = last
        ? last.tokens.input + last.tokens.output + last.tokens.reasoning + last.tokens.cache.read + last.tokens.cache.write
        : undefined
      const contextLimit = last
        ? api.state.provider.find((provider) => provider.id === last.providerID)?.models[last.modelID]?.limit.context
        : undefined
      const attach = process.argv.indexOf("attach")
      const server = api.server ?? (attach >= 0 ? process.argv[attach + 1] : "")
      const status = sessionID ? api.state.session.status(sessionID) : undefined
      const awaitingAnswer = sessionID ? api.state.session.question(sessionID).length > 0 : false
      return {
        pid: process.pid,
        observed_at: Date.now(),
        id: sessionID ?? "",
        title: session?.title ?? "OpenCode",
        directory: session?.directory ?? api.state.path.directory,
        server: server ?? "",
        tab_control: tabControl?.receipt,
        last_question: lastQuestions.get(sessionID),
        activity: awaitingAnswer
          ? "awaiting_answer"
          : status?.type === "busy" || status?.type === "retry" ? "busy" : "idle",
        agent: reply?.agent,
        agent_color: reply?.agent ? api.state.config.agent?.[reply.agent]?.color : undefined,
        model: reply ? `${reply.providerID}/${reply.modelID}` : undefined,
        model_name: model?.name,
        provider_name: provider?.name,
        variant: reply?.variant,
        context_tokens: contextTokens,
        context_limit: contextLimit,
        zellij_session: process.env.ZELLIJ_SESSION_NAME ?? "",
        pane_id: /^\d+$/.test(process.env.ZELLIJ_PANE_ID ?? "") ? Number(process.env.ZELLIJ_PANE_ID) : null,
      }
    }
    const snapshot = () => {
      if (!api.state.ready) return null
      const route = api.route.current
      const sessionID = route.name === "session" ? route.params?.sessionID : undefined
      const record = recordFor(sessionID)
      if (!record) return
      if (api.tabs?.enabled()) {
        const tabs = api.tabs.list()
        const index = tabs.findIndex((tab) => tab.sessionID === sessionID)
        if (index >= 0) record.tab_index = index
        record.tabs = tabs
          .map((tab, index) => tab.sessionID === sessionID ? undefined : { ...recordFor(tab.sessionID), tab_index: index })
          .filter((tab) => tab?.id)
          .filter(Boolean)
          .map((tab) => ({ ...tab, active: false, tab_control: undefined }))
      }
      return record
    }
    const publish = async (record) => {
      if (stopped) return
      if (!record) {
        await rm(file, { force: true })
        return
      }
      await mkdir(root, { recursive: true, mode: 0o700 })
      await writeFile(temporary, JSON.stringify(record), { mode: 0o600 })
      await rename(temporary, file)
    }
    const read = () => {
      try { return snapshot() } catch { return undefined }
    }
    const tick = (record = read()) => {
      if (record === undefined) return pending
      // A failed bridge must never interrupt the conversation or write to the terminal.
      pending = pending.then(() => publish(record)).catch(() => {}).then(initializeConversation).catch(() => {})
      return pending
    }
    let changeTimer
    let latest
    const changed = (record) => {
      if (stopped || record === undefined) return
      latest = record
      if (changeTimer) return
      changeTimer = setTimeout(() => {
        changeTimer = undefined
        tick(latest)
      }, 25)
    }
    const stopWatch = api.lifecycle.watch?.(read, changed)
    const stopEvents = api.lifecycle.listen?.(() => changed(read()))
    // A heartbeat maintains the private navigation lease; native changes publish immediately.
    const timer = setInterval(tick, stopWatch ? 5000 : 1000)
    timer.unref?.()
    api.lifecycle.onDispose(async () => {
      stopped = true
      initialRequest.abort()
      clearInterval(timer)
      clearTimeout(changeTimer)
      stopWatch?.()
      stopEvents?.()
      await tabControl?.close()
      await pending
      await Promise.all([rm(file, { force: true }), rm(temporary, { force: true })])
    })
    await tick()
  },
}

async function serveTabs(api, disposed) {
  const token = randomBytes(32).toString("hex")
  let busy = false
  const server = createServer(async (request, response) => {
    const reply = (status, body) => {
      response.writeHead(status, { "Content-Type": "application/json" })
      response.end(JSON.stringify(body))
    }
    if (request.headers.authorization !== `Bearer ${token}`) return reply(401, { error: "Unauthorized" })
    const focusing = request.url === "/tabs/focus"
    if (request.method !== "POST" || (!focusing && request.url !== "/tabs")) return reply(404, { error: "Unknown action" })
    if (busy) return reply(409, { error: "OpenCode tab creation is already in progress" })
    if (!focusing && !api.tabs.enabled()) return reply(409, { error: "Enable OpenCode session tabs before creating a tab" })
    busy = true
    const cancelled = new AbortController()
    response.on("close", () => cancelled.abort())
    const signal = AbortSignal.any([disposed, cancelled.signal, AbortSignal.timeout(10_000)])
    try {
      let body = ""
      request.setEncoding("utf8")
      for await (const chunk of request) {
        body += chunk
        if (Buffer.byteLength(body) > 16_384) throw new Error("Tab request is too large")
      }
      const { directory, sessionID, instructions } = JSON.parse(body)
      if (instructions != null && typeof instructions !== "string") return reply(400, { error: "Invalid session instructions" })
      if (focusing) {
        const current = api.route.current
        const selected = current.name === "session" && current.params?.sessionID === sessionID
        const open = api.tabs.enabled() && api.tabs.list().some((tab) => tab.sessionID === sessionID)
        if (!selected && !open) return reply(409, { error: "The OpenCode tab closed; refresh and try again" })
        signal.throwIfAborted()
        if (api.tabs.enabled() && api.tabs.focus(sessionID) === false) {
          return reply(409, { error: "OpenCode refused tab navigation" })
        }
        return reply(200, { id: sessionID })
      }
      if (typeof directory !== "string" || !isAbsolute(directory) || !api.state.ready) {
        return reply(400, { error: "OpenCode workspace is unavailable" })
      }
      signal.throwIfAborted()
      for (const tab of api.tabs.list()) {
        const id = tab.sessionID
        if (!api.state.session.get(id)) await api.state.session.sync(id)
        const empty = () => api.state.session.get(id)?.directory === directory
          && api.state.session.status(id)?.type === "idle"
          && api.state.session.messages(id).length === 0
          && api.state.session.pending(id).length === 0
          && api.state.session.question(id).length === 0
        if (!empty()) continue
        // An unloaded background cache is not evidence that a conversation is empty.
        await Promise.all([
          api.state.session.syncMessages(id),
          api.state.session.syncPending(id),
          api.state.session.syncQuestions(id),
        ])
        signal.throwIfAborted()
        if (!empty() || !api.tabs.list().some((tab) => tab.sessionID === id)) continue
        await applyInstructions(api, id, directory, instructions, { throwOnError: true, signal })
        signal.throwIfAborted()
        if (api.tabs.focus(id) === false) return reply(409, { error: "OpenCode refused tab navigation" })
        return reply(200, { id })
      }
      signal.throwIfAborted()
      const result = await api.client.session.create({ directory }, { throwOnError: true, signal })
      signal.throwIfAborted()
      if (!result.data?.id) throw new Error("Session creation returned no session")
      await applyInstructions(api, result.data.id, directory, instructions, { throwOnError: true, signal })
      signal.throwIfAborted()
      api.tabs.focus(result.data.id)
      reply(200, { id: result.data.id })
    } catch {
      if (!response.destroyed) reply(502, { error: focusing
        ? "Could not select the OpenCode tab; refresh and try again"
        : "Could not create an OpenCode tab; check the client before retrying because creation may be uncertain" })
    } finally {
      busy = false
    }
  })
  server.requestTimeout = 10_000
  server.headersTimeout = 10_000
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolve)
  })
  server.unref()
  return {
    receipt: { server: `http://127.0.0.1:${server.address().port}`, token },
    close: () => new Promise((resolve) => {
      server.close(resolve)
      server.closeAllConnections()
    }),
  }
}

async function applyInstructions(api, sessionID, directory, instructions, options) {
  if (!instructions) return
  if (api.client.session.attachInstructions) {
    await api.client.session.attachInstructions(sessionID, instructions, options)
  } else {
    // V1 has no instruction entries; synthetic no-reply context never starts a model request.
    await api.client.session.prompt({
      directory, sessionID, noReply: true, parts: [{ type: "text", text: instructions, synthetic: true }],
    }, options)
  }
}

function legacyMessage(message) {
  return {
    ...message,
    role: message.type,
    providerID: message.model?.providerID,
    modelID: message.model?.id,
    variant: message.model?.variant,
    tokens: message.tokens ?? { input: 0, output: 0, reasoning: 0, cache: { read: 0, write: 0 } },
  }
}

export default plugin
