#!/usr/bin/env bash
# Writes dist/THIRD-PARTY.txt: the licence and copyright notice of every crate in
# the release binary (cargo-about, about.toml), and checks that egui's fonts are in it.
set -euo pipefail
. "$(dirname "$0")/env.sh"
cd "$ROOT"
have=$(cargo about --version 2>/dev/null | awk '{print $2}' || true)
if [ "$have" != "$CARGO_ABOUT_VERSION" ]; then
    echo "need cargo-about $CARGO_ABOUT_VERSION (have: ${have:-none}):" >&2
    echo "  cargo install cargo-about --locked --features cli --version $CARGO_ABOUT_VERSION" >&2
    exit 1
fi
mkdir -p dist
cargo about generate --locked --fail -c about.toml packaging/about.hbs -o dist/THIRD-PARTY.txt
for want in 'UBUNTU FONT LICENCE' 'BITSTREAM VERA' 'Copyright (c) 2014 John Slegers' 'SIL OPEN FONT LICENSE'; do
    if ! grep -qi -- "$want" dist/THIRD-PARTY.txt; then
        echo "dist/THIRD-PARTY.txt has no \"$want\": check the clarify in about.toml" >&2
        exit 1
    fi
done
echo "dist/THIRD-PARTY.txt: $(grep -c '^- ' dist/THIRD-PARTY.txt) entries"
