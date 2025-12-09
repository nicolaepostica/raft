# Shared settings of the packaging scripts: source it, don't run it.
# shellcheck shell=bash

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CARGO_ABOUT_VERSION=0.9.3
APP=raft
# The version lives in one place: [workspace.package] in Cargo.toml.
VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml")

# copy_docs DEST: the licences and notices every package carries.
# Fails when dist/THIRD-PARTY.txt (packaging/licences.sh) is missing.
copy_docs() {
    local dest=$1
    mkdir -p "$dest"
    if [ ! -s "$ROOT/dist/THIRD-PARTY.txt" ]; then
        echo "copy_docs: dist/THIRD-PARTY.txt is missing: run packaging/licences.sh" >&2
        return 1
    fi
    cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/NOTICE" "$ROOT/dist/THIRD-PARTY.txt" "$dest/"
}
