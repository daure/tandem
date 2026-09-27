import { mkdir, rename, rm, writeFile } from "node:fs/promises"
import { homedir } from "node:os"
import { join } from "node:path"

// Runs in each TUI, not in the shared server: route and terminal identity are client-local.
export default {
  id: "tandem.presence",
  async tui(api, options = {}) {
    let initialPrompt = process.env.TANDEM_INITIAL_PROMPT
    delete process.env.TANDEM_INITIAL_PROMPT
    const root = options.presenceDirectory ?? join(
      process.env.XDG_STATE_HOME ?? join(homedir(), ".local/state"), "tandem/opencode",
    )
    const file = join(root, `${process.pid}.json`)
    const temporary = `${file}.tmp`
    let stopped = false
    let pending = Promise.resolve()
    const initialRequest = new AbortController()

    const initializeConversation = async () => {
      if (stopped || !api.state.ready || initialPrompt === undefined) return
      const text = initialPrompt
      // Consume before awaiting: an uncertain HTTP result must never trigger a second submission.
      initialPrompt = undefined
      if (!text.trim()) return
      try {
        if (api.route.current.name !== "home") throw new Error("Client already has a conversation")
        const directory = api.state.path.directory
        const options = {
          throwOnError: true,
          signal: AbortSignal.any([initialRequest.signal, AbortSignal.timeout(15_000)]),
        }
        const result = await api.client.session.create({ directory }, options)
        if (stopped) return
        if (!result.data?.id) throw new Error("Session creation returned no session")
        if (api.route.current.name !== "home") throw new Error("Client changed conversations")
        api.route.navigate("session", { sessionID: result.data.id })
        await api.client.session.promptAsync({
          directory,
          sessionID: result.data.id,
          parts: [{ type: "text", text }],
        }, options)
      } catch {
        if (!stopped) api.ui.toast({
          variant: "error",
          title: "Tandem initial prompt",
          message: "Could not submit the initial prompt. Check this conversation before retrying; delivery may be uncertain.",
        })
      }
    }

    const publish = async () => {
      if (stopped) return
      const route = api.route.current
      const sessionID = route.name === "session" ? route.params?.sessionID : undefined
      if (!api.state.ready) {
        await rm(file, { force: true })
        return
      }
      const session = sessionID ? api.state.session.get(sessionID) : undefined
      if (sessionID && !session) return
      const messages = sessionID ? api.state.session.messages(sessionID) : []
      const question = messages.findLast((message) => message.role === "user")
      const questionText = question
        ? api.state.part(question.id)
          .filter((part) => part.type === "text")
          .map((part) => part.text)
          .join(" ")
          .replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/gu, "")
          .replace(/\s+/gu, " ").trim()
        : ""
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
      const server = attach >= 0 ? process.argv[attach + 1] : ""
      const status = sessionID ? api.state.session.status(sessionID) : undefined
      const awaitingAnswer = sessionID ? api.state.session.question(sessionID).length > 0 : false
      const record = {
        pid: process.pid,
        observed_at: Date.now(),
        id: sessionID ?? "",
        title: session?.title ?? "OpenCode",
        directory: session?.directory ?? api.state.path.directory,
        server: server ?? "",
        last_question: questionText ? Array.from(questionText).slice(0, 4096).join("") : undefined,
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
      await mkdir(root, { recursive: true, mode: 0o700 })
      await writeFile(temporary, JSON.stringify(record), { mode: 0o600 })
      await rename(temporary, file)
    }
    const tick = () => {
      // A failed bridge must never interrupt the conversation or write to the terminal.
      pending = pending.then(publish).catch(() => {}).then(initializeConversation).catch(() => {})
      return pending
    }
    const timer = setInterval(tick, 1000)
    timer.unref?.()
    api.lifecycle.onDispose(async () => {
      stopped = true
      initialRequest.abort()
      clearInterval(timer)
      await pending
      await Promise.all([rm(file, { force: true }), rm(temporary, { force: true })])
    })
    await tick()
  },
}
