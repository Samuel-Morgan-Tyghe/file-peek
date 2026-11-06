#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
if [[ ! -f node_modules/dompurify/dist/purify.min.js ]]; then
  npm ci --ignore-scripts --no-audit --no-fund
fi
app="$PWD/build/File Peek.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
export MACOSX_DEPLOYMENT_TARGET=13.0
cargo build --release --locked
cp target/release/FilePeek "$app/Contents/MacOS/FilePeek"
cp Resources/Info.plist "$app/Contents/Info.plist"
cp Resources/preview.* "$app/Contents/Resources/"
cp node_modules/dompurify/dist/purify.min.js "$app/Contents/Resources/"
cp node_modules/dompurify/LICENSE "$app/Contents/Resources/DOMPurify-LICENSE"
python3 scripts/third_party_notices.py "$app/Contents/Resources/THIRD_PARTY_NOTICES.txt"
cp LICENSE "$app/Contents/Resources/File-Peek-LICENSE"
cp Resources/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
rm -f "$app/Contents/Resources/marked.umd.js" "$app/Contents/Resources/Marked-LICENSE" "$app/Contents/Resources/Marked-LICENSE.md"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
printf 'Built: %s\n' "$app"
