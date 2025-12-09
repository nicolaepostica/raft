#!/usr/bin/env bash
# packaging/notes.sh VERSION: prints the CHANGELOG.md section of VERSION (the release
# notes) without its heading; fails when the section is missing or empty.
set -euo pipefail
. "$(dirname "$0")/env.sh"
v=${1:?usage: notes.sh VERSION}
notes=$(awk -v v="$v" '
    index($0, "## [" v "]") == 1 { on = 1; next }
    on && (/^## \[/ || /^\[[^]]+\]: /) { exit }
    on { print }' "$ROOT/CHANGELOG.md" | sed -e '/./,$!d')
if [ -z "${notes//[[:space:]]/}" ]; then
    echo "CHANGELOG.md has no section \"## [$v]\"" >&2
    exit 1
fi
printf '%s\n' "$notes"
