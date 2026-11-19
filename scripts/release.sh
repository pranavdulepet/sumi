#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
npm run bundle -- --bundles app
built=src-tauri/target/release/bundle/macos/Sumi.app
stage=$(mktemp -d "${TMPDIR:-/tmp}/sumi-release-XXXXXXXX")
archive=$(mktemp website/.sumi-package-XXXXXXXX)
trap 'rm -rf "$stage"; test ! -f "$archive" || unlink "$archive"' EXIT
app="$stage/Sumi.app"
/usr/bin/ditto --norsrc --noextattr "$built" "$app"
resources="$app/Contents/Resources"
mkdir -p "$resources/licenses"
cp LICENSE "$resources/licenses/Sumi.txt"
cp licenses/*.txt "$resources/licenses/"
python3 scripts/release-notices.py "$resources/licenses/Dependencies.txt"
for binary in "$resources/runtime/"*.dylib "$resources/runtime/llama-server"; do
  codesign --force --sign - --timestamp=none "$binary"
done
codesign --force --sign - --timestamp=none "$app"
codesign --verify --deep --strict "$app"
/usr/bin/ditto -c -k --keepParent --norsrc --noextattr "$app" "$archive"
unzip -tq "$archive"
/usr/bin/ditto -x -k "$archive" "$stage/extracted"
codesign --verify --deep --strict "$stage/extracted/Sumi.app"
chmod 644 "$archive"
mv -f "$archive" website/Sumi-macOS.zip
shasum -a 256 website/Sumi-macOS.zip
/usr/bin/ditto --norsrc --noextattr "$app" "$built"
codesign --verify --deep "$built"
python3 scripts/source-archive.py
