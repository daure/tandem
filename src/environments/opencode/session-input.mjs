export async function selectModel(api, directory, name, selectedVariant, options, requireSelection = true) {
  const [modelID, embeddedVariant] = name?.split("#") ?? []
  let variant = selectedVariant ?? embeddedVariant
  const [providerID, ...modelPath] = modelID?.split("/") ?? []
  let model = name ? { providerID, id: modelPath.join("/"), ...(variant ? { variant } : {}) } : undefined
  if (!model && api.ui.selectedModel) {
    // Capture the client's choice before focusing the newly created conversation.
    const selected = api.ui.selectedModel()
    if (!selected && requireSelection) throw new Error("OpenCode's selected model is unavailable")
    if (selected) {
      variant ??= selected.variant
      model = { providerID: selected.providerID, id: selected.modelID, ...(variant ? { variant } : {}) }
    }
  }
  if (!model && variant && api.client.session.defaultModel) {
    model = { ...await api.client.session.defaultModel(directory, options), variant }
  }
  return { model, variant }
}

function receipt(api, sessionID = null) {
  const attach = process.argv.indexOf("attach")
  return {
    session_id: sessionID,
    server: api.server ?? (attach >= 0 ? process.argv[attach + 1] : ""),
    prompt_outcome: "not_submitted", error: null,
  }
}

export async function promptFreshSession(api, input, signal) {
  const outcome = receipt(api)
  let creating = false
  const options = { throwOnError: true, signal }
  try {
    if (input.agent) await api.client.session.validateAgent(input.directory, input.agent, options)
    const { model, variant } = await selectModel(api, input.directory, input.model, input.variant, options)
    signal.throwIfAborted()
    creating = true
    const result = await api.client.session.create({ directory: input.directory, ...(model ? { model } : {}), ...(input.agent ? { agent: input.agent } : {}) }, options)
    outcome.session_id = result.data?.id ?? null
    if (!outcome.session_id) throw new Error("Session creation returned no session")
    signal.throwIfAborted()
    await applyInstructions(api, outcome.session_id, input.directory, input.instructions, options)
    signal.throwIfAborted()
    focusSession(api, outcome.session_id)
    signal.throwIfAborted()
    outcome.prompt_outcome = "uncertain"
    const response = await api.client.session.promptAsync({
      directory: input.directory, sessionID: outcome.session_id,
      parts: [{ type: "text", text: input.initial_prompt }],
      ...(model ? { model: { providerID: model.providerID, modelID: model.id } } : {}),
      ...(variant ? { variant } : {}),
      ...(input.agent ? { agent: input.agent } : {}),
    }, options)
    if (response?.error) throw new Error("OpenCode refused initial prompt submission")
    outcome.prompt_outcome = "submitted"
  } catch {
    outcome.error = outcome.prompt_outcome === "uncertain"
      ? "Prompt delivery may be uncertain; inspect this conversation before retrying"
      : outcome.session_id
        ? "Session setup failed; initial prompt was not submitted"
        : creating
          ? "Session creation may be uncertain; initial prompt was not submitted"
          : "Model selection failed; initial prompt was not submitted"
  }
  return outcome
}

function focusSession(api, id, existing = false) {
  if (api.tabs?.enabled()) {
    if (api.tabs.focus(id) === false) throw new Error("OpenCode refused tab navigation")
  } else {
    const route = api.route.current
    if (route.name !== "home" && !(existing && route.params?.sessionID === id)) throw new Error("Client changed conversations")
    api.route.navigate("session", { sessionID: id })
  }
}

async function verifySession(api, input, options) {
  const session = await api.client.session.get(input.sessionID, options)
  if (session.id !== input.sessionID || session.location.directory !== input.directory) {
    throw new Error("OpenCode conversation ownership changed")
  }
  return session
}

export async function promptExistingSession(api, input, signal) {
  const outcome = receipt(api, input.sessionID)
  const options = { throwOnError: true, signal }
  let mutated = false
  try {
    const session = await verifySession(api, input, options)
    const busy = await api.client.session.busy(session, options)
    if (busy && input.when_busy === "abort") {
      outcome.error = "OpenCode conversation is busy or awaiting an answer; prompt was not submitted"
      return outcome
    }
    if (input.agent) await api.client.session.validateAgent(input.directory, input.agent, options)
    if (busy && input.when_busy === "interrupt") {
      signal.throwIfAborted()
      mutated = true
      await api.client.session.interrupt(input.sessionID, options)
      if (await api.client.session.busy(session, options)) throw new Error("Conversation is still awaiting work or an answer")
    }
    signal.throwIfAborted()
    focusSession(api, input.sessionID, true)
    await applyInstructions(api, input.sessionID, input.directory, input.instructions, options)
    const hasModelOverride = input.model != null || input.variant != null
    if (input.agent != null) {
      mutated = true
      await api.client.session.switchAgent(input.sessionID, input.agent, options)
    }
    if (hasModelOverride) {
      const [modelID, embeddedVariant] = input.model?.split("#") ?? []
      const variant = input.variant ?? embeddedVariant
      const [providerID, ...modelPath] = modelID?.split("/") ?? []
      const model = input.model
        ? { providerID, id: modelPath.join("/"), ...(variant ? { variant } : {}) }
        : { ...session.model, variant }
      if (!model.providerID || !model.id) throw new Error("Conversation has no selected model for the variant override")
      mutated = true
      await api.client.session.switchModel(input.sessionID, model, options)
    }
    await verifySession(api, input, options)
    if (input.when_busy === "abort" && await api.client.session.busy(session, options)) {
      outcome.error = "OpenCode conversation became busy; prompt was not submitted. Session setup may have applied"
      return outcome
    }
    signal.throwIfAborted()
    outcome.prompt_outcome = "uncertain"
    const delivery = input.when_busy === "interrupt" ? "steer" : "queue"
    const response = await api.client.session.promptAsync({
      sessionID: input.sessionID, parts: [{ type: "text", text: input.initial_prompt }], delivery, resume: true,
    }, options)
    if (response?.error) throw new Error("OpenCode refused prompt submission")
    outcome.prompt_outcome = delivery === "queue" ? "queued" : "submitted"
  } catch {
    outcome.error = outcome.prompt_outcome === "uncertain"
      ? "Prompt delivery may be uncertain; inspect this conversation before retrying"
      : `Session setup failed; prompt was not submitted${mutated ? ". Settings or interruption may have applied; inspect before retrying" : ""}`
  }
  return outcome
}

export async function applyInstructions(api, sessionID, directory, instructions, options) {
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
