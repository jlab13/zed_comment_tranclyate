# Repository Guidelines

## Language Requirements

- Always communicate with the user in Ukrainian.
- Write code comments, including documentation comments, in Ukrainian.
- Write commit messages in English.

## Project Structure & Module Organization

This Cargo workspace contains the Zed WASM extension in `src/lib.rs` and the native LSP in `server/src/`. `extension.toml` registers C support. Tree-sitter identifies comments. `server/src/google.rs` implements portable Google HTTP translation; `native/` contains the statically linked Swift Apple backend. `examples/` contains settings; `scripts/` contains build and smoke tests.

Apple is the default on macOS with feature `apple`; Google is the default on Linux and builds without that feature. Never fall back from Apple to Google automatically. Only `cfg(all(target_os = "macos", feature = "apple"))` may reference Swift or Apple frameworks. `build.rs` must return early for other targets.

## Build, Test, and Development Commands

- `./scripts/build.sh`: build the native server and WASM extension on macOS or Linux.
- `cargo build --release -p comment-translate --target wasm32-wasip2`: build the extension.
- `cargo test --workspace`: run Rust unit tests.
- `cargo test -p comment-translate-lsp --no-default-features`: test without Apple.
- `./scripts/test_native.sh`: check Swift and CLI on macOS after building.
- `cargo fmt --all --check`: check formatting.
- `cargo clippy --workspace --all-targets -- -D warnings`: check lints.
- `cargo build -p comment-translate-lsp` followed by `python3 scripts/smoke_test.py`: run the LSP smoke test without system language packs.

Install the extension through Zed's `zed: install dev extension` action. See `README.md` for setup. Do not build the native server for WASM.

## Coding Style & Naming Conventions

For Rust code, use standard `rustfmt` formatting with four-space indentation. Use four-space indentation in Swift as well. Name modules and functions in `snake_case`, types and traits in `PascalCase`, and constants in `SCREAMING_SNAKE_CASE`. Prefer descriptive names and small, cohesive functions.

## Testing Guidelines

Use Rust's built-in test framework alongside implementation. Name tests after observable behavior. Cover comment boundaries, strings containing comment markers, Unicode positions, document changes, native failures, and caching. No coverage threshold is configured. Tests must not require language packs or remote services. Ignored loopback HTTP tests require local socket permission; Linux CI runs them with `--include-ignored`.

## Commit & Pull Request Guidelines

Use concise, imperative English commit subjects, such as `Add Google translation provider`. Keep each commit focused.

Pull requests should explain the change, its purpose, and validation performed. Link relevant issues and include screenshots when a change affects visible behavior. Explicitly note any checks that could not be run.
