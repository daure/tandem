#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
binary=${1:-$root/target/release/tandem}
bin=${2:-${CARGO_HOME:-$HOME/.cargo}/bin}
mkdir -p "$bin"
bin=$(CDPATH='' cd -- "$bin" && pwd -P)
. "$root/scripts/upgrade-processes.sh"
upgrade_prepare "$bin/tandem" "$binary"
install -m 755 "$binary" "$bin/tandem.new"
mv -f "$bin/tandem.new" "$bin/tandem"
"$bin/tandem" opencode-setup
upgrade_finish
printf 'Installed %s\n' "$bin/tandem"
