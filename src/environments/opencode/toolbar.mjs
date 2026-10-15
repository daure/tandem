import { companionBinary, watchState, openInstance, openRoute } from "./toolbar-transport.mjs"
import { renderInstanceDot } from "./toolbar-view.mjs"

export function scopeEnvironment(documents, options = {}, inherited = process.env) {
  const env = { ...inherited }
  // Read only the explicitly configured Tandem scope, never execute config commands.
  for (const document of documents ?? []) {
    const server = document.info?.mcp?.servers?.["tandem-instance"]
    if (server?.type !== "local" || server.disabled || !server.command?.includes("mcp-instance")) continue
    for (const key of ["TANDEM_HOME", "TANDEM_NAMESPACE", "TANDEM_GATEWAY_PORT"]) {
      if (typeof server.environment?.[key] === "string") env[key] = server.environment[key]
    }
  }
  if (options.tandemHome) env.TANDEM_HOME = options.tandemHome
  if (options.tandemNamespace) env.TANDEM_NAMESPACE = options.tandemNamespace
  return env
}

export function instanceView(state, server, sessionID) {
  const session = state?.sessions.get(JSON.stringify([server, sessionID]))
  const instance = session?.instance && state.instances.get(session.instance.toLowerCase())
  if (!instance) return
  const stale = !state.connected || session.stale || instance.runtime?.stale
    || Boolean(state.metadata.instances?.runtime_error || state.metadata.sessions?.observation_error || state.metadata.sessions?.inventory_error)
  return {
    instance,
    stale,
    label: stale ? "Stale" : instance.workspace_only ? "Workspace only" : instance.summary.label,
    severity: stale ? "warning" : instance.summary.severity,
    services: instance.workspace_only ? "" : `${instance.summary.running}/${instance.summary.expected} services running`,
    routes: instance.services.filter((service) => service.url).map((service) => ({ title: service.name, value: service.url, description: service.url })),
    disabled: Boolean(stale || instance.pending || instance.summary.busy),
  }
}

function instanceActionOptions(instance) {
  const { running, expected, status } = instance.summary
  return [
    ...(!instance.workspace_only ? [
      {
        title: "Start services", value: "start", category: "Services",
        description: "Apply the trusted template and wait for readiness",
        disabled: expected > 0 && running >= expected,
      },
      {
        title: "Stop services", value: "stop", category: "Services",
        description: "Preserve the workspace, volumes, and networks",
        disabled: ["not_started", "stopped", "completed"].includes(status),
      },
    ] : []),
    { title: "Purge instance…", value: "purge", description: "Permanently delete the workspace and instance data", category: "Destructive" },
  ]
}

export async function setupToolbar(context, server, dependencies = {}) {
  const reactive = dependencies.reactive ?? await import("solid-js")
  const elements = dependencies.elements ?? await import("@opentui/solid")
  const binary = context.options.tandemBinary ?? await companionBinary()
  const { createRoot, createSignal, createEffect } = reactive
  const watch = dependencies.watchState ?? watchState
  const connect = dependencies.openInstance ?? openInstance
  const browse = dependencies.openRoute ?? openRoute
  let disposeRoot, stopWatch, disposed = false, generation = 0, selection = "", environment
  let activeControl
  const abort = new AbortController()
  const unregister = []
  const dispose = () => {
    disposed = true
    generation++
    abort.abort()
    activeControl?.close()
    stopWatch?.()
    for (const stop of unregister) stop()
    disposeRoot?.()
  }
  try {
    createRoot((stop) => {
      disposeRoot = stop
      const [state, setState] = createSignal()
      const [busy, setBusy] = createSignal(false)
      const [menuOpen, setMenuOpen] = createSignal(false)
      const currentSession = () => {
        const route = context.ui.router.current()
        return route.type === "session" ? route.sessionID : undefined
      }
      const current = () => instanceView(state(), server, currentSession())
      const settings = () => ({ binary, env: environment, signal: abort.signal })
      const notify = (message, variant = "error") => {
        if (!disposed) context.ui.toast.show({ title: "Tandem instance", message, variant })
      }
      const stillSelected = (id, target) => {
        const view = current()
        return !disposed && currentSession() === id && view && !view.disabled
          && view.instance.name === target.name && view.instance.workspace === target.workspace
          && view.instance.template === target.template
      }
      const actions = async () => {
        const view = current(), id = currentSession()
        if (!view || view.disabled || busy() || menuOpen()) return
        const target = view.instance
        setMenuOpen(true)
        let control
        try {
          // Pin the workspace before the menu or confirmation can outlive its incarnation.
          control = await connect(target, settings())
          activeControl = control
          if (!stillSelected(id, target)) throw new Error("The selected instance changed; reopen the menu")
          const action = await context.ui.dialog.select({
            title: `${target.name} · instance actions`,
            options: instanceActionOptions(current().instance),
          })
          if (!action) return
          if (!["start", "stop", "purge"].includes(action)) throw new Error("Unknown Tandem action")
          if (action === "purge" && !await context.ui.dialog.confirm({
            title: `Purge ${target.name}?`,
            message: `Permanently delete ${target.workspace}, its containers, volumes, and networks. Associated OpenCode clients and panes will close. Save any work first; this cannot be undone. Conversation history and saved memories remain.`,
            label: { confirm: "Purge instance", cancel: "Keep instance" },
          })) return
          if (!stillSelected(id, target)) throw new Error("The selected instance changed; reopen the menu")
          const option = instanceActionOptions(current().instance).find((option) => option.value === action)
          if (!option || option.disabled) throw new Error("The selected action is unavailable; reopen the menu")
          setBusy(true)
          notify(action === "purge" ? "Requesting purge…" : `${action === "start" ? "Starting" : "Stopping"} ${target.name}…`, "info")
          await control.action(action)
          notify(action === "purge" ? "Purge admitted; cleanup continues independently." : `${target.name}: services ${action === "start" ? "ready" : "stopped"}`, "success")
        } catch (error) { notify(error.message) }
        finally {
          control?.close()
          activeControl = undefined
          if (!disposed) { setBusy(false); setMenuOpen(false) }
        }
      }
      const routes = async () => {
        const view = current(), id = currentSession()
        if (!view || view.disabled || busy() || menuOpen() || !view.routes.length) return
        setMenuOpen(true)
        try {
          const url = await context.ui.dialog.select({ title: `${view.instance.name} · routes`, placeholder: "Search routes…", options: view.routes })
          if (!url) return
          if (!stillSelected(id, view.instance) || !current().routes.some((route) => route.value === url)) {
            throw new Error("The instance routes changed; reopen the menu")
          }
          await browse(url)
        } catch (error) { notify(error.message) }
        finally { if (!disposed) setMenuOpen(false) }
      }
      createEffect(() => {
        const id = currentSession()
        const directory = id && context.data.session.get(id)?.location.directory
        const next = JSON.stringify([id, directory])
        if (selection === next) return
        selection = next
        const revision = ++generation
        activeControl?.close()
        stopWatch?.()
        stopWatch = undefined
        setState(undefined)
        if (!directory) return
        void (async () => {
          try {
            const response = await context.client.config.get({ location: { directory } }, { signal: AbortSignal.any([abort.signal, AbortSignal.timeout(10_000)]) })
            if (disposed || generation !== revision) return
            environment = scopeEnvironment(response, context.options)
            stopWatch = watch({ ...settings(), autoStart: context.options.autoStartState !== false }, (value) => {
              if (!disposed && generation === revision) setState({ ...value })
            })
          } catch (error) { if (!disposed && generation === revision) notify(`Instance controls unavailable: ${error.message}`) }
        })()
      })
      unregister.push(context.ui.slot({
        before: "prompt.footer.status",
        render: () => renderInstanceDot(context, current, reactive, elements),
      }))
      unregister.push(context.ui.slot({
        append: "app",
        render: () => {
          context.keymap.layer(() => ({
            mode: "global",
            priority: 10,
            commands: [
              { id: "tandem.instance.actions", title: "Tandem: Instance actions", bind: "<leader>i", palette: true, group: "Tandem", enabled: () => Boolean(current() && !current().disabled && !busy() && !menuOpen()), run: actions },
              { id: "tandem.instance.routes", title: "Tandem: Open instance route", bind: "<leader>r", palette: true, group: "Tandem", enabled: () => Boolean(current()?.routes.length && !current().disabled && !busy() && !menuOpen()), run: routes },
            ],
            bindings: ["tandem.instance.actions", "tandem.instance.routes"],
          }))
          return null
        },
      }))
    })
    return dispose
  } catch (error) { dispose(); throw error }
}
