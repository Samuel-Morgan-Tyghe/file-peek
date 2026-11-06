#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
app="$PWD/build/File Peek.app"
codesign --verify --strict "$app"
version=$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")
mkdir -p build/release
archive="File-Peek-${version}-macOS-$(uname -m).zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "build/release/$archive"
cd build/release
shasum -a 256 "$archive" > SHA256SUMS.txt
printf 'Packaged: %s\n' "$archive"
