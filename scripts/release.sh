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
identity=${SUMI_SIGN_IDENTITY:--}
if [[ -n "${SUMI_NOTARY_PROFILE:-}" && "$identity" == "-" ]]; then
  echo "Choose a Developer ID Application identity for notarization." >&2
  exit 1
fi
sign_flags=(--force --sign "$identity")
if [[ "$identity" == "-" ]]; then
  sign_flags+=(--timestamp=none)
else
  sign_flags+=(--options runtime --timestamp)
fi
for binary in "$resources/runtime/"*.dylib "$resources/runtime/llama-server"; do
  codesign "${sign_flags[@]}" "$binary"
done
codesign "${sign_flags[@]}" "$app"
if [[ -n "${SUMI_NOTARY_PROFILE:-}" ]]; then
  /usr/bin/ditto -c -k --keepParent --norsrc --noextattr "$app" "$stage/submission.zip"
  xcrun notarytool submit "$stage/submission.zip" --keychain-profile "$SUMI_NOTARY_PROFILE" --wait
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
fi
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
