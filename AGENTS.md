# Repository Guidelines

## Language Requirements

- Always communicate with the user in Ukrainian.
- Write code comments, including documentation comments, in Ukrainian.
- Write user-facing server text (hover, errors, logs, CLI) in English with a Ukrainian variant through `i18n::tr!`; Ukrainian is shown only when the target language is `uk`.
- Write commit messages in English.

## Project Structure & Module Organization

This Cargo workspace contains the Zed WASM extension in `src/lib.rs` and the native LSP in `server/src/`. `extension.toml` registers C, C++, Rust, Python, Shell Script, Swift, and Dart; `server/src/languages.rs` describes each language. Tree-sitter identifies comments. `server/src/locale.rs` resolves the target language from `target_language` or the system locale. `server/src/segments.rs` keeps code blocks and inline code out of translation. `server/src/google.rs` implements portable Google HTTP translation; `server/src/mymemory.rs` implements the MyMemory fallback; `server/src/deepl.rs` implements DeepL on the user's own key. `examples/` contains settings and demo files; `scripts/` contains build and smoke tests.

The server is pure Rust with no platform-specific backends and no cargo features; Google is the default provider on every platform. By default DeepL falls back to Google and MyMemory, and Google to MyMemory; `fallback` overrides the order. Never put the DeepL key in Zed settings, URLs, or logs.

## Build, Test, and Development Commands

- `./scripts/build.sh`: build the native server and WASM extension on macOS or Linux.
- `cargo build --release -p comment-translate --target wasm32-wasip2`: build the extension.
- `cargo test --workspace`: run Rust unit tests.
- `python3 scripts/test_cli.py target/release/comment-translate-lsp`: check the CLI after building.
- `cargo fmt --all --check`: check formatting.
- `cargo clippy --workspace --all-targets -- -D warnings`: check lints.
- `cargo build -p comment-translate-lsp` followed by `python3 scripts/smoke_test.py`: run the LSP smoke test without system language packs.

Install the extension through Zed's `zed: install dev extension` action. See `README.md` for setup. Do not build the native server for WASM.

## Coding Style & Naming Conventions

For Rust code, use standard `rustfmt` formatting with four-space indentation. Name modules and functions in `snake_case`, types and traits in `PascalCase`, and constants in `SCREAMING_SNAKE_CASE`. Prefer descriptive names and small, cohesive functions.

## Testing Guidelines

Use Rust's built-in test framework alongside implementation. Name tests after observable behavior. Cover comment boundaries, strings containing comment markers, Unicode positions, document changes, provider failures, and caching. No coverage threshold is configured. Tests must not require language packs or remote services. Ignored loopback HTTP tests require local socket permission; Linux CI runs them with `--include-ignored`.

## Commit & Pull Request Guidelines

Use concise, imperative English commit subjects, such as `Add Google translation provider`. Keep each commit focused.

Pull requests should explain the change, its purpose, and validation performed. Link relevant issues and include screenshots when a change affects visible behavior. Explicitly note any checks that could not be run.
