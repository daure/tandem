# Tandem

Template-based development workspaces for developers and coding agents.

[![Release](https://img.shields.io/github/v/release/daure/tandem)](https://github.com/daure/tandem/releases/latest)
[![Build](https://github.com/daure/tandem/actions/workflows/release.yml/badge.svg)](https://github.com/daure/tandem/actions/workflows/release.yml)

[Install](#install) · [First workspace](#create-your-first-workspace) · [MCP](#connect-an-agent) · [Reference](docs/reference.md)

Manage repositories, Docker Compose services, and OpenCode sessions through a TUI, CLI, or MCP.

## Features

- Reusable templates for repositories, services, and agent guidance.
- Service health, resource usage, routes, and agent sessions in one view.
- Event rules that route incoming work to OpenCode.

## Install

**Ubuntu 24.04+, x86_64.** Use a [Nerd Font](https://www.nerdfonts.com/) for icons.

```sh
installer="$(mktemp)"
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/daure/tandem/releases/latest/download/tandem-installer.sh \
  -o "$installer" && sh "$installer"
rm -f "$installer"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
tandem
```

The installer includes the OpenCode companion and prints its configuration path. See [OpenCode setup](docs/reference.md#opencode-integration).

Blank workspaces need no Git or Docker. Repository templates require Linux and Git; service templates require Docker Engine and Compose v2+. OpenCode panes require Zellij 0.45+ and the companion.

## Create your first workspace

1. Open **Instances**, press `T`, and create a template named `website`. Its `tandem.json` starts with `{}`.
2. Select the template, press `i`, and name the instance `review`. Confirm to prepare the workspace.
3. Select the instance and press `d` to inspect its path and agent guidance.

These are the default shortcuts. Add repositories and services through [template configuration](docs/reference.md#template-layout-and-routing).

With the template in place, use the CLI:

```sh
tandem new-instance another-review --template website
tandem list-instances
tandem inspect-instance another-review
```

Run only trusted templates; scripts and containers use host and Docker privileges.
Stopping keeps data. Purging permanently deletes the workspace and owned resources.

## Connect an agent

Configure a local stdio MCP server:

```json
{
  "command": "tandem",
  "args": ["mcp"]
}
```

Agents call `get_instructions` first. Use [instance-scoped MCP](docs/reference.md#instance-local-control) for workspace-local operations.

## Documentation and contributing

- [TUI and workspace lifecycle](docs/reference.md#quick-start)
- [CLI commands](docs/reference.md#cli-instance-lifecycle) · [Configuration](docs/reference.md#configuration)
- [Templates and routing](docs/reference.md#template-layout-and-routing)
- [OpenCode integration](docs/reference.md#opencode-integration)
- [Events and providers](docs/reference.md#events-and-developer-providers) · [Rules](docs/reference.md#event-rules)
- [Local state stream](docs/reference.md#local-state-stream-and-session-origins)
- [Architecture](architecture.md) · [Verification](docs/reference.md#verification) · [Releases](docs/reference.md#release)
- [Issues](https://github.com/daure/tandem/issues)

Build with Rust and a sibling `../tuicore` checkout: `cargo run`. See [development fixtures](projects-generators/README.md) for sample apps.

## License

MIT or Apache-2.0, at your option, as declared in [Cargo.toml](Cargo.toml).
