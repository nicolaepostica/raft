#!/usr/bin/env bash
# packaging/version.sh          prints the version (Cargo.toml, [workspace.package])
# packaging/version.sh set X.Y.Z  changes it (and Cargo.lock)
set -euo pipefail
. "$(dirname "$0")/env.sh"
if [ "${1:-}" = set ]; then
    new=${2:?usage: version.sh set X.Y.Z}
    [[ $new =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || { echo "bad version: $new" >&2; exit 1; }
    sed -i "/^\[workspace.package\]/,/^\[/s/^version *= *\".*\"/version = \"$new\"/" "$ROOT/Cargo.toml"
    (cd "$ROOT" && cargo update --workspace -q)
    echo "$new"
else
    echo "$VERSION"
fi
