#!/usr/bin/env bash
# Linux packages from target/release/raft:
#   dist/raft-<version>-linux-x86_64.tar.gz, dist/raft_<version>_amd64.deb and
#   dist/raft-<version>-1.x86_64.rpm (Fedora, openSUSE, RHEL)
# Needs dist/THIRD-PARTY.txt (packaging/licences.sh), dpkg-deb for the .deb and rpmbuild
# (package `rpm` on Ubuntu, `rpm-build` on Fedora) for the .rpm; a missing tool skips its package.
set -euo pipefail
. "$(dirname "$0")/env.sh"
cd "$ROOT"
bin=target/release/$APP
[ -x "$bin" ] || { echo "missing $bin: cargo build --release -p raft" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# --- tar.gz: unpack anywhere and run ./raft -------------------------------------
name=$APP-$VERSION-linux-x86_64
t=$work/$name
mkdir -p "$t"
install -m 755 "$bin" scripts/build-catalog scripts/rename-books "$t/"
install -m 644 assets/raft.desktop "$t/raft.desktop"
install -m 644 assets/icon.png "$t/raft.png"
install -m 644 README.md CHANGELOG.md "$t/"
copy_docs "$t"
tar -C "$work" --owner=0 --group=0 -czf "dist/$name.tar.gz" "$name"
echo "dist/$name.tar.gz"

# The installed tree, shared by the .deb and the .rpm: $1 = its root.
stage() {
    local d=$1
    install -Dm 755 "$bin" "$d/usr/bin/$APP"
    install -Dm 755 scripts/build-catalog "$d/usr/lib/$APP/build-catalog"
    install -Dm 755 scripts/rename-books "$d/usr/lib/$APP/rename-books"
    ln -s "../lib/$APP/build-catalog" "$d/usr/bin/build-catalog"
    ln -s "../lib/$APP/rename-books" "$d/usr/bin/rename-books"
    install -Dm 644 assets/raft.desktop "$d/usr/share/applications/$APP.desktop"
    install -Dm 644 assets/icon.png "$d/usr/share/icons/hicolor/256x256/apps/$APP.png"
    copy_docs "$d/usr/share/doc/$APP"
}

# --- .rpm ---------------------------------------------------------------------
# Before the .deb, which ends the script when dpkg-deb is missing.
if command -v rpmbuild >/dev/null; then
    rpm_version=${VERSION/-/\~}      # 1.0.0~rc.1 sorts before 1.0.0
    r=$work/rpm
    stage "$r/root"
    install -m 644 CHANGELOG.md "$r/root/usr/share/doc/$APP/CHANGELOG.md"
    cat >"$r/$APP.spec" <<SPEC
Name: $APP
Version: $rpm_version
Release: 1
Summary: Personal book library with search, filters and a reader
License: MIT OR Apache-2.0
URL: https://github.com/nicolaepostica/raft
BuildArch: x86_64
# Dependencies are named by hand: the binary is built on Ubuntu (glibc 2.35).
AutoReqProv: no
Requires: glibc >= 2.35, libxkbcommon, poppler-utils, djvulibre, python3, python3-pillow
Recommends: libxkbcommon-x11, libwayland-client, mesa-libEGL, vulkan-loader, glib2, zenity
Suggests: tesseract, tesseract-langpack-rus
%global debug_package %{nil}
%global __os_install_post %{nil}

%description
Raft shows a folder of PDF, DjVu, EPUB and FB2 books as a searchable library:
full-text search, filters by language, format, year, category, series and
folder, reading progress, hand corrections of the metadata, and readers for
every format. build-catalog makes the folder's catalog.

%install
cp -a $r/root/. %{buildroot}/

%files
/usr/bin/$APP
/usr/bin/build-catalog
/usr/bin/rename-books
/usr/lib/$APP
/usr/share/applications/$APP.desktop
/usr/share/icons/hicolor/256x256/apps/$APP.png
%dir /usr/share/doc/$APP
%license /usr/share/doc/$APP/LICENSE-MIT
%license /usr/share/doc/$APP/LICENSE-APACHE
%license /usr/share/doc/$APP/NOTICE
%license /usr/share/doc/$APP/THIRD-PARTY.txt
%doc /usr/share/doc/$APP/CHANGELOG.md
SPEC
    rpmbuild -bb --quiet --define "_topdir $r/top" --define "_rpmdir $ROOT/dist" \
        --define "source_date_epoch_from_changelog 0" \
        --define "_build_name_fmt %%{NAME}-%%{VERSION}-%%{RELEASE}.%%{ARCH}.rpm" \
        "$r/$APP.spec"
    echo "dist/$APP-$rpm_version-1.x86_64.rpm"
else
    echo "no rpmbuild: skipping the .rpm" >&2
fi

# --- .deb ---------------------------------------------------------------------
command -v dpkg-deb >/dev/null || { echo "no dpkg-deb: skipping the .deb" >&2; exit 0; }
deb_version=${VERSION/-/\~}          # 1.0.0-rc.1 sorts before 1.0.0
d=$work/deb
stage "$d"
doc=$d/usr/share/doc/$APP
gzip -9n -c CHANGELOG.md >"$doc/changelog.gz"
{
    echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
    echo "Upstream-Name: Raft"
    echo "Source: https://github.com/nicolaepostica/raft"
    echo
    echo "Files: *"
    echo "Copyright: $(sed -n 's/^Copyright (c) //p' LICENSE-MIT | head -1)"
    echo "License: MIT or Apache-2.0"
    echo "Comment: The Rust crates built into /usr/bin/raft, egui's fonts among them,"
    echo " are listed with their full licence texts in /usr/share/doc/raft/THIRD-PARTY.txt."
    echo
    echo "License: MIT"
    sed -n '/^Permission is hereby granted/,$p' LICENSE-MIT | sed 's/^$/./; s/^/ /'
    echo
    echo "License: Apache-2.0"
    echo " On Debian systems the full text of the Apache License, Version 2.0 is in"
    echo " /usr/share/common-licenses/Apache-2.0, and in /usr/share/doc/raft/LICENSE-APACHE."
} >"$doc/copyright"
size=$(du -sk "$d/usr" | cut -f1)
mkdir -p "$d/DEBIAN"
cat >"$d/DEBIAN/control" <<CONTROL
Package: $APP
Version: $deb_version
Architecture: amd64
Maintainer: Nicolae Postica <nicolaepostica@users.noreply.github.com>
Installed-Size: $size
Depends: libc6 (>= 2.35), libxkbcommon0, poppler-utils, djvulibre-bin, python3, python3-pil
Recommends: libxkbcommon-x11-0, libwayland-client0, libegl1, libvulkan1, libglib2.0-bin, zenity, tesseract-ocr, tesseract-ocr-rus
Section: text
Priority: optional
Homepage: https://github.com/nicolaepostica/raft
Description: personal book library with search, filters and a reader
 Raft shows a folder of PDF, DjVu, EPUB and FB2 books as a searchable library:
 full-text search, filters by language, format, year, category, series and
 folder, reading progress, hand corrections of the metadata, and readers for
 every format. build-catalog makes the folder's catalog.
CONTROL
find "$d" -type d -exec chmod 755 {} +
dpkg-deb --root-owner-group -Zxz --build "$d" "dist/${APP}_${deb_version}_amd64.deb" >/dev/null
echo "dist/${APP}_${deb_version}_amd64.deb"
