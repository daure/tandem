# Tandem

**Repeatable workspaces for you and your coding agents.**

[![Release](https://img.shields.io/github/v/release/daure/tandem)](https://github.com/daure/tandem/releases/latest)
[![Build](https://github.com/daure/tandem/actions/workflows/release.yml/badge.svg)](https://github.com/daure/tandem/actions/workflows/release.yml)

[Install](#install) · [First workspace](#create-your-first-workspace) · [MCP](#connect-an-agent) · [Reference](docs/reference.md)

Tandem turns editable templates into local development workspaces. Manage repositories, Docker Compose services, and OpenCode sessions through a keyboard-first TUI, CLI, or MCP.

## What you get

- **Repeatable environments:** create workspaces from shared repository, service, and guidance templates.
- **A workspace overview:** inspect service health, resource usage, routes, and agent sessions.
- **Agent workflows:** connect MCP clients or use event rules to route incoming work to OpenCode.

## Install

Prebuilt binaries target **Ubuntu 24.04+ on x86_64**. Use a terminal with a [Nerd Font](https://www.nerdfonts.com/) for icons.

```sh
installer="$(mktemp)"
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/daure/tandem/releases/latest/download/tandem-installer.sh \
  -o "$installer" && sh "$installer"
rm -f "$installer"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
tandem
```

The installer also installs the OpenCode companion and prints its plugin configuration path.
See [OpenCode setup and updates](docs/reference.md#opencode-integration) for configuration and client restarts.

Blank workspaces need neither Git nor Docker. Repository templates require Linux and host Git; service templates require Docker Engine and Compose v2+. OpenCode pane integration requires Zellij 0.45+ and the companion.

## Create your first workspace

1. Open **Instances** and press `T` to create a template named `website`. A blank template starts with `tandem.json` containing `{}`.
2. Select it, press `i`, and name the instance `review`. Confirm the dialog to prepare its workspace.
3. Select the instance and press `d` to inspect its workspace path and generated agent guidance.

These are the default bindings; the TUI shows your configured shortcuts.
Add repositories or Compose services to the template when needed; see [template authoring](docs/reference.md#template-layout-and-routing).

Once a template exists, the CLI can create and inspect instances too:

```sh
tandem new-instance another-review --template website
tandem list-instances
tandem inspect-instance another-review
```

Only execute trusted templates: their scripts and containers use local host and Docker privileges.
Stopping preserves workspace data; purging permanently deletes the instance's workspace and owned resources.

## Connect an agent

Configure a local stdio MCP server:

```json
{
  "command": "tandem",
  "args": ["mcp"]
}
```

Agents call `get_instructions` first. Tandem also provides an [instance-scoped MCP server](docs/reference.md#instance-local-control) for workspace-local operations.

## Documentation and contributing

- [TUI controls and workspace lifecycle](docs/reference.md#quick-start)
- [CLI commands](docs/reference.md#cli-instance-lifecycle) · [Configuration](docs/reference.md#configuration)
- [Templates and routing](docs/reference.md#template-layout-and-routing)
- [OpenCode integration](docs/reference.md#opencode-integration)
- [Events and providers](docs/reference.md#events-and-developer-providers) · [Automation rules](docs/reference.md#event-rules)
- [Local state stream](docs/reference.md#local-state-stream-and-session-origins)
- [Architecture](architecture.md) · [Verification](docs/reference.md#verification) · [Releasing](docs/reference.md#release)
- [Report a bug or request a feature](https://github.com/daure/tandem/issues)

Source builds require Rust and a sibling `../tuicore` checkout. Start with `cargo run`; see the [development fixtures](projects-generators/README.md) for sample applications.

## License

MIT or Apache-2.0, at your option, as declared in [Cargo.toml](Cargo.toml).
