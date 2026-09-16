#!/bin/sh
set -eu

if [ "${FAIL_CLONE:-0}" = 1 ]; then
    printf '%s\n' 'Injected clone failure' >&2
    exit 1
fi

for name in "$@"; do
    source="$SOURCE_ROOT/$name"
    target="/workspace/$name"
    if [ -e "$target" ] || [ -L "$target" ]; then
        if [ -L "$target" ] || [ ! -d "$target/.git" ]; then
            printf '%s\n' "Refusing non-repository checkout: $target" >&2
            exit 1
        fi
        origin=$(git -C "$target" remote get-url origin)
        if [ "$origin" != "$source" ]; then
            printf '%s\n' "Unexpected origin in $target: $origin" >&2
            exit 1
        fi
        printf '%s\n' "Preserving $target"
    else
        git -c "safe.directory=$source/.git" clone --no-local --branch main "$source" "$target"
    fi
done
