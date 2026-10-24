#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
case "$(uname -s)" in
    Darwin) cargo build --release -p comment-translate-lsp "$@" ;;
    Linux) cargo build --release -p comment-translate-lsp "$@" ;;
    *) echo "Підтримуються macOS та Linux." >&2; exit 1 ;;
esac
cargo build --release -p comment-translate --target wasm32-wasip2
