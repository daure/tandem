import { mkdir, rename, rm, writeFile } from "node:fs/promises"
import { homedir } from "node:os"
import { join } from "node:path"

// Runs in each TUI, not in the shared server: route and terminal identity are client-local.
export default {
  id: "tandem.presence",
  async tui(api, options = {}) {
    const root = options.presenceDirectory ?? join(
      process.env.XDG_STATE_HOME ?? join(homedir(), ".local/state"), "tandem/opencode",
    )
    const file = join(root, `${process.pid}.json`)
    const temporary = `${file}.tmp`
    let stopped = false
    let pending = Promise.resolve()

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
      const last = messages.findLast((message) => message.role === "assistant" && message.tokens.output > 0)
      const contextTokens = last
        ? last.tokens.input + last.tokens.output + last.tokens.reasoning + last.tokens.cache.read + last.tokens.cache.write
        : undefined
      const contextLimit = last
        ? api.state.provider.find((provider) => provider.id === last.providerID)?.models[last.modelID]?.limit.context
        : undefined
      const attach = process.argv.indexOf("attach")
      const server = attach >= 0 ? process.argv[attach + 1] : ""
      const status = sessionID ? api.state.session.status(sessionID) : undefined
      const record = {
        pid: process.pid,
        observed_at: Date.now(),
        id: sessionID ?? "",
        title: session?.title ?? "OpenCode",
        directory: session?.directory ?? api.state.path.directory,
        server: server ?? "",
        activity: status?.type === "busy" || status?.type === "retry" ? "busy" : "idle",
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
      pending = pending.then(publish).catch(() => {})
      return pending
    }
    const timer = setInterval(tick, 1000)
    timer.unref?.()
    api.lifecycle.onDispose(async () => {
      stopped = true
      clearInterval(timer)
      await pending
      await Promise.all([rm(file, { force: true }), rm(temporary, { force: true })])
    })
    await tick()
  },
}
