import { execFile, spawn } from "node:child_process"
import { readFile } from "node:fs/promises"

const MAX_FRAME = 16 * 1024 * 1024

export async function companionBinary() {
  try {
    const settings = JSON.parse(await readFile(new URL("./companion.json", import.meta.url), "utf8"))
    if (typeof settings.binary !== "string" || !settings.binary) throw new Error("Invalid Tandem companion executable")
    return settings.binary
  } catch (error) {
    if (error.code === "ENOENT") return "tandem"
    throw error
  }
}

export function readFrames(stream, receive, failed) {
  let buffer = ""
  stream.setEncoding("utf8")
  const data = (chunk) => {
    buffer += chunk
    try {
      let end
      while ((end = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, end)
        buffer = buffer.slice(end + 1)
        if (Buffer.byteLength(line) > MAX_FRAME) throw new Error("Tandem frame exceeds 16 MiB")
        if (line.trim()) receive(JSON.parse(line))
      }
      if (Buffer.byteLength(buffer) > MAX_FRAME) throw new Error("Tandem frame exceeds 16 MiB")
    } catch (error) { failed(error) }
  }
  stream.on("data", data)
  return () => stream.off("data", data)
}

export function watchState({ binary = "tandem", env = process.env, autoStart = true }, changed, launch = spawn) {
  let instances = new Map(), sessions = new Map(), metadata = {}, connected = false
  let child, retry, stopped = false, started = false, delay = 1000
  const publish = () => changed({ instances, sessions, metadata, connected })
  const key = (session) => JSON.stringify([session.server, session.session_id])
  const receive = (frame) => {
    if (frame.version !== 1) throw new Error("Unsupported Tandem state protocol")
    switch (frame.type) {
      case "snapshot":
        instances = new Map(frame.data.instances.instances.map((value) => [value.name.toLowerCase(), value]))
        sessions = new Map(frame.data.sessions.sessions.map((value) => [key(value), value]))
        metadata = { instances: frame.data.instances, sessions: frame.data.sessions }
        connected = true
        started = false
        delay = 1000
        break
      case "instance.changed": instances.set(frame.data.name.toLowerCase(), frame.data); break
      case "instance.removed": instances.delete(frame.data.name.toLowerCase()); break
      case "session.created":
      case "session.changed": sessions.set(key(frame.data), frame.data); break
      case "session.removed": sessions.delete(key(frame.data)); break
      case "state.changed": metadata = frame.data; break
      default: return
    }
    publish()
  }
  const connect = () => {
    if (stopped) return
    const process = launch(binary, ["watch", "--json"], { env, stdio: ["ignore", "pipe", "ignore"] })
    child = process
    let ended = false
    const disconnected = () => {
      if (ended) return
      ended = true
      removeFrames()
      process.kill()
      connected = false
      publish()
      if (stopped) return
      // The listener's namespace lease elects one shared collector across clients.
      if (autoStart && !started) {
        started = true
        const server = launch(binary, ["serve-state"], { env, detached: true, stdio: "ignore" })
        server.on("error", () => {})
        server.unref()
      }
      retry = setTimeout(connect, delay)
      retry.unref?.()
      delay = Math.min(delay * 2, 10_000)
    }
    const removeFrames = readFrames(process.stdout, receive, disconnected)
    process.once("error", disconnected)
    process.once("close", disconnected)
  }
  connect()
  return () => {
    stopped = true
    clearTimeout(retry)
    child?.kill()
  }
}

export async function openInstance(target, { binary = "tandem", env = process.env, signal }, launch = spawn) {
  signal?.throwIfAborted()
  const child = launch(binary, ["mcp-instance"], { cwd: target.workspace, env, stdio: ["pipe", "pipe", "ignore"] })
  let sequence = 0, closed = false
  const pending = new Map()
  const fail = (error) => {
    for (const request of pending.values()) request.reject(error)
    pending.clear()
  }
  const close = () => {
    if (closed) return
    closed = true
    signal?.removeEventListener("abort", close)
    removeFrames()
    fail(new Error("Tandem connection closed; an operation may still be running. Inspect before retrying."))
    child.stdin.end()
    child.kill()
  }
  const removeFrames = readFrames(child.stdout, (frame) => {
    const request = pending.get(frame.id)
    if (!request) return
    pending.delete(frame.id)
    if (frame.error) request.reject(new Error(frame.error.message))
    else request.resolve(frame.result)
  }, (error) => { fail(error); close() })
  child.once("error", (error) => { fail(error); close() })
  child.once("close", close)
  child.stdin.on("error", (error) => { fail(error); close() })
  signal?.addEventListener("abort", close, { once: true })
  const send = (payload) => child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", ...payload })}\n`)
  const request = (method, params, milliseconds = 15_000) => new Promise((resolve, reject) => {
    if (closed) return reject(new Error("Tandem connection is closed"))
    const id = ++sequence
    const timer = setTimeout(() => {
      reject(new Error("Tandem response timed out; inspect the instance before retrying."))
      close()
    }, milliseconds)
    pending.set(id, {
      resolve: (value) => { clearTimeout(timer); resolve(value) },
      reject: (error) => { clearTimeout(timer); reject(error) },
    })
    send({ id, method, params })
  })
  const tool = async (name, milliseconds) => {
    const result = await request("tools/call", { name, arguments: {} }, milliseconds)
    if (result.isError) throw new Error(result.content?.find((part) => part.type === "text")?.text ?? "Tandem action failed")
    return result.structuredContent ?? JSON.parse(result.content.find((part) => part.type === "text").text)
  }
  try {
    await request("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "tandem-toolbar", version: "1" } })
    send({ method: "notifications/initialized" })
    const identity = await tool("get_instructions")
    if (identity.instance !== target.name || identity.workspace !== target.workspace || identity.template !== target.template) {
      throw new Error("Tandem instance identity changed; reopen the menu")
    }
    return {
      close,
      async action(action) {
        const name = { start: "start_self", stop: "stop_self", purge: "purge" }[action]
        if (!name) throw new Error("Unknown Tandem action")
        return tool(name, action === "start" ? 630_000 : 90_000)
      },
    }
  } catch (error) { close(); throw error }
}

function browserDesktop(protocol) {
  return new Promise((resolve) => {
    execFile("xdg-mime", ["query", "default", `x-scheme-handler/${protocol.slice(0, -1)}`], { timeout: 5000, maxBuffer: 4096 }, (error, stdout) => {
      const desktop = stdout.trim()
      resolve(!error && /^[A-Za-z0-9][A-Za-z0-9_.-]*\.desktop$/.test(desktop) ? desktop : undefined)
    })
  })
}

export async function openRoute(url, launch = spawn, resolveBrowser = browserDesktop, platform = process.platform) {
  const parsed = new URL(url)
  if (!["http:", "https:"].includes(parsed.protocol) || parsed.username || parsed.password) {
    throw new Error("Tandem routes must be HTTP(S) URLs without credentials")
  }
  const open = (command, args, options = {}) => new Promise((resolve, reject) => {
    const child = launch(command, args, { stdio: "ignore", timeout: 15_000, ...options })
    child.once("error", reject)
    child.once("exit", (code) => code === 0 ? resolve() : reject(new Error("Could not open the route in your browser")))
  })
  if (platform === "win32") {
    return open("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
      "Start-Process -FilePath $env:TANDEM_ROUTE_URL -Verb Open -ErrorAction Stop"], {
      env: { ...process.env, TANDEM_ROUTE_URL: parsed.href }, windowsHide: true,
    })
  }
  if (platform === "linux") {
    const desktop = await resolveBrowser(parsed.protocol)
    if (desktop) {
      // GTK supplies the desktop activation context needed to transfer focus on Wayland.
      try { return await open("gtk-launch", [desktop, parsed.href]) }
      catch (error) { if (error.code !== "ENOENT") throw error }
    }
  }
  return open(platform === "darwin" ? "open" : "xdg-open", [parsed.href])
}
