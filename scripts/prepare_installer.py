#!/usr/bin/env python3

import argparse
from pathlib import Path


MARKER = '''    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "everything's installed!"'''

COMPANION_SETUP = '''    ignore rm -rf "$_install_temp" "$_lib_install_temp"

    say "installing OpenCode TUI companion"
    ensure "$_install_dir/tandem" opencode-setup

    say "everything's installed!"'''


def prepare(path: Path) -> None:
    source = path.read_text()
    if COMPANION_SETUP in source:
        return
    if source.count(MARKER) != 1:
        raise RuntimeError("cargo-dist shell installer layout changed")
    path.write_text(source.replace(MARKER, COMPANION_SETUP))


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Add Tandem's OpenCode companion setup to a cargo-dist shell installer"
    )
    parser.add_argument("installer", type=Path)
    args = parser.parse_args()
    prepare(args.installer)


if __name__ == "__main__":
    main()
