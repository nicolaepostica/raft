#!/usr/bin/env bash
# dist/raft-<version>-x86_64.AppImage from target/release/raft: one file that runs on most
# distributions. Needs dist/THIRD-PARTY.txt (packaging/licences.sh) and curl. appimagetool and
# the AppImage runtime are downloaded at pinned versions and checked against their sha256.
# Not bundled (as with the other packages): poppler-utils, djvulibre, python3 with Pillow.
set -euo pipefail
. "$(dirname "$0")/env.sh"
cd "$ROOT"
bin=target/release/$APP
[ -x "$bin" ] || { echo "missing $bin: cargo build --release -p raft" >&2; exit 1; }

APPIMAGETOOL_URL=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
RUNTIME_URL=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

tools=$ROOT/target/appimage-tools
mkdir -p "$tools"
fetch() {   # fetch URL SHA256 FILE: download once, always verify
    local url=$1 sum=$2 file=$3
    if [ ! -s "$file" ]; then
        curl -fsSL --retry 3 -o "$file.part" "$url"
        mv "$file.part" "$file"
    fi
    echo "$sum  $file" | sha256sum -c --quiet - || { rm -f "$file"; echo "checksum mismatch: $url" >&2; exit 1; }
}
fetch "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256" "$tools/appimagetool"
fetch "$RUNTIME_URL" "$RUNTIME_SHA256" "$tools/runtime-x86_64"
chmod +x "$tools/appimagetool"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
a=$work/$APP.AppDir
# build-catalog and rename-books next to the binary, where raft looks for them first.
install -Dm 755 "$bin" "$a/usr/bin/$APP"
install -m 755 scripts/build-catalog scripts/rename-books "$a/usr/bin/"
install -Dm 644 assets/raft.desktop "$a/usr/share/applications/$APP.desktop"
install -Dm 644 assets/icon.png "$a/usr/share/icons/hicolor/256x256/apps/$APP.png"
copy_docs "$a/usr/share/doc/$APP"
install -m 644 CHANGELOG.md "$a/usr/share/doc/$APP/"
cp assets/raft.desktop "$a/$APP.desktop"
cp assets/icon.png "$a/$APP.png"
ln -s "$APP.png" "$a/.DirIcon"
cat >"$a/AppRun" <<'APPRUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/raft" "$@"
APPRUN
chmod 755 "$a/AppRun"

out=dist/$APP-$VERSION-x86_64.AppImage
# Extract-and-run: no FUSE needed on the build machine (CI runners have none).
if ! ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tools/appimagetool" --no-appstream \
    --runtime-file "$tools/runtime-x86_64" "$a" "$out" >"$work/appimagetool.log" 2>&1; then
    cat "$work/appimagetool.log" >&2
    exit 1
fi
echo "$out"
