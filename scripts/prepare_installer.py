#!/usr/bin/env python3

import argparse
from pathlib import Path


MARKER = '''    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "everything's installed!"'''

MOVE = '    for _bin_name in $_bins; do\n        ensure mv "$_install_temp/$_bin_name" "$_install_dir"\n'
ENTRY = 'download_binary_and_run_installer "$@" || exit 1\n'

COMPANION_SETUP = '''    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "installing OpenCode TUI companion"
    ensure "$_install_dir/tandem" opencode-setup

    say "everything's installed!"'''

POST_INSTALL = COMPANION_SETUP.replace(
    '    say "everything\'s installed!"',
    '''    ensure upgrade_finish

    say "everything's installed!"''',
)


def prepare(path: Path) -> None:
    source = path.read_text()
    if POST_INSTALL in source:
        return
    marker = COMPANION_SETUP if COMPANION_SETUP in source else MARKER
    if any(source.count(anchor) != 1 for anchor in (marker, MOVE, ENTRY)):
        raise RuntimeError("cargo-dist shell installer layout changed")
    hooks = (Path(__file__).parent / "upgrade-processes.sh").read_text()
    source = source.replace(MOVE, '    ensure upgrade_prepare "$_install_dir/tandem" "$_install_temp/tandem"\n' + MOVE)
    source = source.replace(ENTRY, hooks + "\n" + ENTRY)
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
