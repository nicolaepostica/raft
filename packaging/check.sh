#!/usr/bin/env bash
# packaging/check.sh DIR: opens every package in DIR and fails unless each one has
# the binary, both scripts, the desktop entry, the icon and every licence file
# (.tar.gz, .deb, .rpm, .AppImage).
set -euo pipefail
dir=${1:?usage: check.sh DIR}
docs=(LICENSE-MIT LICENSE-APACHE NOTICE THIRD-PARTY.txt)
fail=0

need() {   # need PACKAGE LISTING PATH...
    local pkg=$1 list=$2; shift 2
    local p
    for p in "$@"; do
        if ! grep -qE "(^|/)${p//./\\.}\$" <<<"$list"; then
            echo "$pkg: missing $p" >&2
            fail=1
        fi
    done
}

found=0
for f in "$dir"/*.tar.gz; do
    [ -e "$f" ] || continue
    found=1
    list=$(tar -tzf "$f")
    need "$f" "$list" raft build-catalog rename-books raft.desktop raft.png "${docs[@]}"
    t=$(mktemp -d)
    tar -xzf "$f" -C "$t"
    grep -q 'UBUNTU FONT LICENCE' "$t"/*/THIRD-PARTY.txt || { echo "$f: THIRD-PARTY.txt without egui's fonts" >&2; fail=1; }
    rm -rf "$t"
done
for f in "$dir"/*.deb; do
    [ -e "$f" ] || continue
    found=1
    list=$(dpkg-deb -c "$f" | awk '{print $6}')
    need "$f" "$list" usr/bin/raft usr/bin/build-catalog usr/bin/rename-books \
        usr/lib/raft/build-catalog usr/share/applications/raft.desktop \
        usr/share/icons/hicolor/256x256/apps/raft.png usr/share/doc/raft/copyright \
        "${docs[@]/#/usr/share/doc/raft/}"
done
for f in "$dir"/*.rpm; do
    [ -e "$f" ] || continue
    found=1
    if ! list=$(rpm -qlp "$f" 2>/dev/null); then
        echo "$f: not a readable rpm" >&2
        fail=1
        continue
    fi
    list=$(sed 's|^/||' <<<"$list")
    need "$f" "$list" usr/bin/raft usr/bin/build-catalog usr/bin/rename-books \
        usr/lib/raft/build-catalog usr/share/applications/raft.desktop \
        usr/share/icons/hicolor/256x256/apps/raft.png "${docs[@]/#/usr/share/doc/raft/}"
    if ! rpm -qp --queryformat '%{LICENSE}' "$f" 2>/dev/null | grep -qx 'MIT OR Apache-2.0'; then
        echo "$f: licence tag is not MIT OR Apache-2.0" >&2
        fail=1
    fi
done
for f in "$dir"/*.AppImage; do
    [ -e "$f" ] || continue
    found=1
    t=$(mktemp -d)
    abs=$(readlink -f "$f")
    if ! (cd "$t" && "$abs" --appimage-extract >/dev/null 2>&1) || [ ! -d "$t/squashfs-root" ]; then
        echo "$f: cannot be extracted" >&2
        fail=1
        rm -rf "$t"
        continue
    fi
    list=$(cd "$t/squashfs-root" && find . -mindepth 1 | sed 's|^\./||')
    need "$f" "$list" AppRun raft.desktop raft.png usr/bin/raft usr/bin/build-catalog \
        usr/bin/rename-books "${docs[@]/#/usr/share/doc/raft/}"
    rm -rf "$t"
done
[ "$found" = 1 ] || { echo "no packages in $dir" >&2; exit 1; }
[ "$fail" = 0 ] && echo "packages in $dir: ok"
exit "$fail"
