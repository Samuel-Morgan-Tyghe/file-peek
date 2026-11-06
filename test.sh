#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
cargo test --locked
"build/File Peek.app/Contents/MacOS/FilePeek" --self-test
