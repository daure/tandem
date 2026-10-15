export function renderInstanceDot(context, current, reactive, elements) {
  const { createComponent, createElement, spread } = elements
  return createComponent(reactive.Show, {
    get when() { return current() },
    children: (_view) => {
      const status = createElement("text")
      spread(status, {
        id: "tandem.instance.health",
        width: 2, height: 1, flexShrink: 0, selectable: false, content: "● ",
        // The host footer adds a two-column gap between slot contributions.
        marginRight: -2,
        get fg() { return context.theme.text.feedback[current()?.severity]?.base ?? context.theme.text.muted },
      })
      return status
    },
  })
}
