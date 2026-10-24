#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    echo "Ці перевірки призначені для Apple-збірки на macOS." >&2
    exit 1
fi
mkdir -p target/swift-module-cache
xcrun swiftc -parse-as-library \
    -module-cache-path target/swift-module-cache \
    native/TranslationStrategy.swift native/tests/StrategyTests.swift \
    -o target/strategy-tests
./target/strategy-tests
python3 scripts/test_apple_translate.py target/release/comment-translate-lsp

