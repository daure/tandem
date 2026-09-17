#!/bin/sh
set -eu

if [ "${1##*/}" = tandem ]; then
    root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
    if [ -f "$root/projects/env.sh" ]; then
        . "$root/projects/env.sh"
    fi
fi

exec "$@"
