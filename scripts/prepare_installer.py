#!/usr/bin/env python3

import argparse
from pathlib import Path


MARKER = '''    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "everything's installed!"'''

COMPANION_SETUP = '''    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "installing OpenCode TUI companion"
    ensure "$_install_dir/tandem" opencode-setup

    say "everything's installed!"'''

POST_INSTALL = COMPANION_SETUP.replace(
    '    say "everything\'s installed!"',
    '''    if command -v systemctl >/dev/null 2>&1; then
        if systemctl --user show-environment >/dev/null 2>&1; then
            if systemctl --user is-active --quiet tandem-mcp.service; then
                say "restarting the active user tandem-mcp.service"
                ensure systemctl --user try-restart tandem-mcp.service
                say "Global MCP connections were interrupted; reconnect clients before issuing more tool calls."
            fi
        else
            say "warning: user service manager unavailable; restart tandem-mcp.service manually if it is running" 1>&2
        fi
    fi

    say "everything's installed!"''',
)


def prepare(path: Path) -> None:
    source = path.read_text()
    if POST_INSTALL in source:
        return
    marker = COMPANION_SETUP if COMPANION_SETUP in source else MARKER
    if source.count(marker) != 1:
        raise RuntimeError("cargo-dist shell installer layout changed")
    path.write_text(source.replace(marker, POST_INSTALL))


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Refresh Tandem's companion and restart its active user MCP service after installation"
    )
    parser.add_argument("installer", type=Path)
    args = parser.parse_args()
    prepare(args.installer)


if __name__ == "__main__":
    main()
