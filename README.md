**English** · [Українська](README_UK.md)

# Code Comment Translation for Zed

An extension for **macOS and Linux** that shows a translation of the comment under the cursor
in Zed's standard hover popup, in the system language (or the language set in `target_language`).
You choose the translation service: **Google Translate** (no API key, falls back to **MyMemory**
when Google does not answer), **MyMemory**, or **DeepL** on your own API key. Until you choose one,
the hover only explains how to do it, and no comment text leaves your machine.
Supports **C, C++, Rust, Python, Shell Script (Bash), Swift, and Dart**: line, block,
and documentation comments, plus Python docstrings. Tree-sitter tells strings, raw strings,
interpolation, and heredocs apart from comments. The code in the file is never modified.

| Zed language | Comments                                        | Notes                                                              |
| ------------ | ----------------------------------------------- | ------------------------------------------------------------------ |
| C, C++       | `//`, `///`, `//!`, `/* */`, `/** */`, `/*! */` | `.h` works in both C and C++ mode                                  |
| Rust         | `//`, `///`, `//!`, `/* */`, `/** */`, `/*! */` | nested block comments                                              |
| Swift        | `//`, `///`, `/* */`, `/** */`                  | nested block comments; the language comes from the Swift extension |
| Dart         | `//`, `///`, `/* */`, `/** */`                  | nested block comments; the language comes from the Dart extension  |
| Python       | `#`, docstrings                                 | a `#!` shebang on the first line is not translated                 |
| Shell Script | `#`                                             | a `#!` shebang on the first line is not translated                 |

## Architecture

```text
Zed → WASM extension → Rust LSP → Google Translate (Rust, all platforms)
                             ↳ MyMemory (Rust, fallback for Google)
                             → DeepL (Rust, own API key; falls back to Google and MyMemory)
```

- `src/lib.rs` — the Zed extension that launches the server.
- `server/src/` — the LSP, comment detection, UTF-16 positions, and the cache.
- `server/src/languages.rs` — the language table: languageId, file extensions, grammar, and markers.
- `server/src/segments.rs` — code protection: splitting into prose and code blocks, labels for inline code.
- `server/src/cache.rs` — the persistent translation cache on disk.
- `server/src/locale.rs` — resolves the target language from the system locale.
- `server/src/google.rs` — the shared Google HTTP client built on reqwest/rustls.
- `server/src/mymemory.rs` — the MyMemory client: request chunking, quota and error handling.
- `server/src/deepl.rs` — the DeepL client: key loading, endpoint by plan, language variants, usage.
- `examples/` — Zed settings, a key binding, and demo files with English comments for every supported language.
- `scripts/` — build and test scripts.

The server is plain Rust with no platform-specific code beyond reading the macOS language list,
and it is built the same way on macOS and Linux. The target language comes from the system locale or from `target_language` (see
[Translation language](#translation-language)). The source language is set with `source_language`;
`auto` lets the translation service detect the language.
If the source language matches the target language, the text is returned unchanged.
Every provider uses the Rust reqwest HTTP client with rustls TLS and needs internet access.
DeepL falls back to Google and MyMemory, and Google to
MyMemory, unless `fallback` says otherwise (see [Fallback order](#fallback-order)).
Translations are cached in memory and on disk (see [Translation cache](#translation-cache)).

## Building

Both platforms need Rust via rustup, a C compiler for Tree-sitter, and access to crates.io
for the first dependency download. On Debian/Ubuntu, install a C compiler with
`sudo apt install build-essential`; on macOS, the Xcode Command Line Tools. OpenSSL is not needed.

```sh
rustup target add wasm32-wasip2
./scripts/build.sh
```

The script builds `target/release/comment-translate-lsp` and the WASM extension. Do not build the
whole workspace for WASM: the LSP is a native process. If you previously set
`initialization_options.helper_path` or `"provider": "apple"`, remove them: the Apple Translation
backend has been removed.

## Choosing a provider

No provider is selected by default. Until `provider` is set, hovering a comment shows how to choose
one, and no comment text is sent anywhere. For Google, add to your Zed settings:

```json
{
  "lsp": {
    "comment-translate": {
      "initialization_options": {
        "provider": "google",
        "source_language": "auto",
        "timeout_seconds": 60,
        "max_comment_chars": 4000
      }
    }
  }
}
```

The other providers are `"mymemory"` and `"deepl"` (see [MyMemory](#mymemory) and [DeepL](#deepl)).
The `target_language` field is described in [Translation language](#translation-language).
Keep your other settings. After switching, restart the language servers or Zed. Examples:
[`zed-settings.json`](examples/zed-settings.json) (Google) and
[`zed-settings-deepl.json`](examples/zed-settings-deepl.json) (DeepL with a key file).

Google works without API keys, but it needs internet access and sends
the comment text to Google. It uses the web endpoint
`https://translate.googleapis.com/translate_a/single` with `client=gtx`, not the paid Cloud
Translation API. This web interface may change or rate-limit requests.
HTTP 403/429, network errors, and malformed JSON are shown in the hover.
Caching and joining lines into paragraphs work for every provider.

Checking Google from the CLI:

```sh
echo '{"provider":"google","text":"Return the next counter value.","source_language":"en"}' \
  | ./target/release/comment-translate-lsp --translate
```

The CLI uses Google when the request has no `provider`, since you type the text yourself.
`--check en` returns `network_required` for Google; verify access to Google
with a real translation through `--translate` with `"provider":"google"`.

### MyMemory

[MyMemory](https://mymemory.translated.net/doc/spec.php) is a free public translation API by
Translated S.r.l. (Italy). It needs no key and is called openly as
`comment-translate-lsp/<version>`. It serves as Google's fallback by default: when Google cannot
be reached, rate-limits requests, or returns an error, the same paragraph goes to MyMemory, and
the reason is written to the language server log. After HTTP 429 or 403 from Google, the server
skips Google for 5 minutes and sends requests straight to MyMemory instead of hitting a service
that is already refusing them. Each service in the chain gets an equal share of
`timeout_seconds`. If every service fails, the hover shows all reasons.

```json
{
  "lsp": {
    "comment-translate": {
      "initialization_options": {
        "provider": "google",
        "fallback": "mymemory",
        "mymemory_email": "you@example.com"
      }
    }
  }
}
```

- `"fallback": "none"` turns fallback off: the comment text is then sent to Google only.
- `"provider": "mymemory"` uses MyMemory directly, without Google.
- Without an address, MyMemory allows about 5,000 characters a day; `mymemory_email` raises
  this to about 50,000. The address is sent to MyMemory with every request; the server never
  fills it in on its own.
- MyMemory accepts at most 500 characters per request, so longer text is split at line,
  sentence, or word boundaries into requests of up to 450 bytes.
- Translation quality is usually lower than Google's. With fallback enabled, comment text may
  reach a second third-party service.

Checking MyMemory from the CLI (the CLI tests the selected service only, without fallback):

```sh
echo '{"provider":"mymemory","text":"Return the next counter value.","source_language":"en"}' \
  | ./target/release/comment-translate-lsp --translate
```

### DeepL

[DeepL](https://www.deepl.com/pro-api) is an optional provider on your own API key; the DeepL API
Free plan has a monthly character quota, and the CLI check below shows yours. With `"provider": "deepl"` the order is
DeepL → Google → MyMemory, the same as in the Omarchy translate plugin.

```json
{
  "lsp": {
    "comment-translate": {
      "initialization_options": {
        "provider": "deepl",
        "deepl_key_file": "~/.local/state/omarchy/translate-deepl.key"
      }
    }
  }
}
```

- The key is never stored in Zed settings: the server reads it from `deepl_key_file` (a leading
  `~/` is expanded) or, without that field, from the `DEEPL_AUTH_KEY` environment variable. The
  key file saved by the Omarchy translate plugin works as is. Keep the file private (`chmod 600`).
- Keys ending in `:fx` (Free plan) go to `api-free.deepl.com`, others to `api.deepl.com`, as a JSON
  POST with the key in the `Authorization` header — never in a URL or the log. The log shows only
  the plan, or why DeepL is skipped.
- The key is read once when the server starts; restart the language server after changing it.
- Without a key, DeepL is skipped and the next service translates. After HTTP 403 (key rejected),
  429, or 456 (monthly quota spent), DeepL is skipped for 5 minutes.
- English and Portuguese targets need a variant on DeepL: `en-GB` or `pt-PT` in `target_language`
  selects British English or European Portuguese, otherwise American English and Brazilian
  Portuguese are used.
- Comment text goes to DeepL SE (Germany).

Checking the key and the month's usage from the CLI:

```sh
echo '{"provider":"deepl","operation":"check","deepl_key_file":"~/.local/state/omarchy/translate-deepl.key"}' \
  | ./target/release/comment-translate-lsp --translate
```

`"status":"ready"` comes with `plan`, `character_count`, and `character_limit`.

### Fallback order

`fallback` sets the services tried after the provider, in order: `"none"`, one provider name, or a
list such as `["google", "mymemory"]`. Without the field, DeepL falls back to Google and MyMemory,
Google to MyMemory, and MyMemory to nothing. The list cannot contain the provider itself or
repeats. The order is written to the language server log at startup.
For example, `"provider": "deepl", "fallback": ["mymemory"]` never sends comments to Google.

## Translation language

The `target_language` field in `initialization_options` sets the translation language as a
BCP 47 code: `"uk"`, `"en"`, `"de"`, `"pl"`, `"zh-Hans"`, `"zh-Hant"`, and so on. Case and `_`
instead of `-` do not matter: `uk_UA` becomes `uk-UA`.

```json
{
  "lsp": {
    "comment-translate": {
      "initialization_options": {
        "target_language": "de"
      }
    }
  }
}
```

If the field is missing or set to `"auto"`, the language is taken from the system when the LSP starts:

- macOS: the first language in **System Settings → General → Language & Region**
  (`defaults read -g AppleLanguages`), then the environment variables;
- Linux: `LC_ALL`, `LC_MESSAGES`, `LANG` in POSIX priority order; a non-empty
  `LANGUAGE` list (for example `uk:en`) takes precedence, except for the `C`/`POSIX` locale;
- if no language can be determined (for example, `LANG=C.UTF-8`), English is used.

The locale region does not affect translation (`uk_UA.UTF-8` → `uk`), but Chinese keeps its
script: `zh_TW` → `zh-Hant`, `zh_CN` → `zh-Hans`. The selected language and its source are
written to the language server log (in Zed's command palette: `open language server logs`).
After changing `target_language` or the system language, restart the language servers or Zed.
The hover header shows the language code: `Translation → de`. The interface — the hover header,
error messages, the language server log, and CLI errors — is in English, and in Ukrainian when the
target language is Ukrainian (`Переклад → uk`).

## Translation cache

Translated paragraphs are kept on disk, so hovering the same comment again — even after restarting
Zed — does not call the service and spends no quota.

- Location: `$XDG_CACHE_HOME/comment-translate`, otherwise `~/Library/Caches/comment-translate` on
  macOS and `~/.cache/comment-translate` on Linux. Directories are readable only by you (`700`).
- Entries are kept for 14 days by default, so that improved translation models eventually
  translate the comment again. Set `"cache_ttl_days"` (1–365) to change that.
- The cache is limited to 50 MB; at startup expired entries and then the oldest ones are removed.
- Each entry is one file named by the SHA-256 of the provider, the source and target language, and
  the paragraph text, so changing the provider or the language gives a new translation.
- Only translations from the chosen provider are stored. An answer from a fallback service stays in
  memory only, so the chosen service translates the paragraph itself once it is available again.
  Errors are never cached.
- The original comment text is not stored, but the translations are, in plain text. Set
  `"cache": false` to keep translations in memory only, and delete the directory to clear the cache.
- In memory, up to 256 paragraphs are kept per server; the oldest ones are evicted first.

```json
{
  "lsp": {
    "comment-translate": {
      "initialization_options": {
        "provider": "deepl",
        "cache_ttl_days": 30
      }
    }
  }
}
```

## Connecting to Zed

The extension does not bundle the language server. It starts the first one it finds:

1. the path in `lsp.comment-translate.binary.path` in your Zed settings;
2. `comment-translate-lsp` on your `PATH`;
3. otherwise, the server downloaded automatically from the GitHub release whose tag matches
   the extension version (extension 0.1.0 uses release `v0.1.0`, so the two always match).
   Prebuilt servers exist for macOS (Apple Silicon and Intel) and Linux (x86_64 and aarch64,
   a static musl build that runs on any distribution). Zed shows the download progress; the
   archive is unpacked into the extension's own working directory, and older versions are
   removed after an update.

On other platforms (Windows, 32-bit x86), or to run your own build, set an absolute path
(a relative one is resolved against the extension's working directory, not the project):

```json
{
  "lsp": {
    "comment-translate": {
      "binary": {
        "path": "/absolute/path/to/comment-translate-lsp"
      }
    }
  }
}
```

To try the extension from source:

1. Run `./scripts/build.sh`.
2. In the Zed command palette, run `zed: install dev extension` and select the project root.
3. Set `binary.path` to the absolute path of `target/release/comment-translate-lsp`, or put the
   binary on your `PATH`; otherwise the extension downloads the released server for its version.
4. Add the `initialization_options` fields you need from
   [`examples/zed-settings.json`](examples/zed-settings.json) to your personal Zed settings.
5. After changing `binary` or `initialization_options`, restart the language servers or Zed.
6. Open a file in a supported language, hover over a comment or place the cursor inside
   a comment and run `editor: hover`. An example `ctrl-alt-t` key binding is in
   [`examples/keymap.json`](examples/keymap.json).

The primary language servers (`clangd`, `rust-analyzer`, `basedpyright`, and so on) work as before:
Zed runs `comment-translate` alongside them through the default
`"language_servers": ["..."]` value and merges their hovers. If a language has its own
`language_servers` list without `"..."`, add `"comment-translate"` to it.
Swift and Dart appear in Zed only after installing the Swift and Dart extensions.
The translator is never called outside comments.
For English-language code you can set `"source_language": "en"`: language detection for
very short comments can be inaccurate. The default is `auto`.

### Releases

The extension downloads only the server release with its own version, so the version must be
the same in `extension.toml`, `Cargo.toml` (`comment-translate`), and `server/Cargo.toml`
(`comment-translate-lsp`); `cargo test` checks this.

1. Bump `version` in all three files, run `cargo check --workspace` so `Cargo.lock` picks up the
   new version, and commit the four files.
2. Tag the commit and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. The [Release workflow](.github/workflows/release.yml) fails if the tag differs from any of the
   three versions. Otherwise it builds `comment-translate-lsp` for `x86_64-unknown-linux-musl`,
   `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`, and publishes
   a GitHub release with `comment-translate-lsp-<target>.tar.gz` archives (the binary at the
   archive root) and `.sha256` checksums.
4. Only after the release is published, point the Zed extension registry at the new commit:
   until then, the new extension version has no server to download.

## C headers (`.h`)

Zed opens `.h` files in **C++** mode by default. The server is registered for both C and C++,
so comments in headers are translated without a `file_types` association. If a project
switches `.h` to **C** through `file_types`, translation works as well.

## Checks

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p comment-translate-lsp
python3 scripts/smoke_test.py
python3 scripts/test_cli.py target/release/comment-translate-lsp
```

The smoke test uses the real LSP with the same source and target language, so it needs no
network. It covers `.c`, `.h` in C and C++ modes, Rust, Python,
Shell Script, ignoring unsupported languages, hover, editing, closing,
limits, and process shutdown. Per-language Rust tests cover comment node kinds,
markers inside strings, line grouping, docstrings, shebangs, and UTF-16 positions. Separate Rust
tests cover caching, errors, timeouts, the fallback chain, and code protection. `test_cli.py`
checks `--check` and `--translate` without network. Rust tests cover Google, MyMemory, and DeepL: parameter encoding, JSON, and HTTP errors. Two loopback HTTP
tests are marked `ignored` because they need permission to open local TCP sockets:

```sh
cargo test -p comment-translate-lsp -- --include-ignored
```

Linux CI runs these checks, builds the server and WASM, and checks the CLI and LSP.
Regular tests need no internet access. Real translation needs access to Google, MyMemory, or
DeepL.

## Code protection

The translator receives only prose; code never reaches it.

- **Code blocks** — Markdown fences (` ``` `, `~~~`), Doxygen `@code … @endcode`,
  `\code … \endcode`, and `@verbatim … @endverbatim`, and Python doctest examples (`>>>`
  up to the next blank line) — are shown verbatim and never sent for translation. The prose
  paragraphs around them are translated and cached separately.
- **Code inside sentences** is replaced with `x0_`, `x1_`, … labels (`x0_()` for a function call, so
  DeepL does not read it as a phone number) before translation and put
  back afterwards: inline code in backticks, identifiers that look like code (`next_value()`,
  `MAX_SIZE`, `nextValue`, `HashMap`, `std::mem::swap`, `node->next`), URLs, Doxygen commands
  with parameter names (`@param[in] node`), and reST fields (`:param count:`). Ordinary words,
  `item(s)`, and email addresses are not touched. Identifier-like labels keep the context:
  Google translates "Call x0_" as a function call, while with `⟦0⟧` it chose a phone call.
- **If the translator loses, changes, or repeats a label**, the paragraph is translated again
  without labels, as before. A paragraph with no words outside code is not sent at all.

Documentation comments in Rust, Swift, and Dart (`///`, `//!`, `/** */`, `/*! */`) are written in
Markdown, so their hover is shown as Markdown: code blocks get syntax highlighting (an untagged
block gets the document language), and line breaks are kept. Regular comments and other languages
are shown as plain text, so `*` and `_` in them are never turned into formatting.

## Limitations

C, C++, Rust, Python, Shell Script, Swift, and Dart are supported on macOS and Linux.
Other languages are ignored: no hover appears for them.
The language is determined by the `languageId` from Zed, or by the file extension if the ID is unknown.
Adjacent line comments (`//`, `///`, `//!`, or `#`) with no code before the marker are
translated as one block. A blank line, code, or a block comment separates blocks.
An empty `//` or `#` comment inside a block keeps paragraphs apart. Before translation,
lines within a paragraph are joined with spaces; paragraph boundaries are kept as a double line break.
List items (`1.`, `1)`, `-`, `*`, `+`, `•`) and Doxygen tags (`@brief`, `@param`,
`@return`, also the `\param` form) stay on separate lines. A wrapped item description is
joined with spaces, and the indentation of nested items is preserved. Markdown headings
(`# Examples`) and reST fields (`:param x:`) stay on separate lines; code blocks are kept
verbatim (see [Code protection](#code-protection)). In nested Rust, Swift, and Dart comments,
only the outer `/*` and `*/` are removed. A Python docstring is only the first string literal
of a module, class, or function without an `f`/`b`/`t` prefix; quotes and `r`/`u` prefixes are
removed, and indentation is normalized as in `inspect.cleandoc`. Other strings, f-strings,
heredocs, and shebangs are not translated. All of this happens before the text is passed to the
selected translator. One translation request runs at a time; requests and queue waits are
limited by `timeout_seconds`; on timeout, the request in flight is cancelled.
There is no dedicated translation command: Zed's standard hover is used, so translation can
also be triggered by hovering the mouse. The extension downloads only its own server from this
repository's GitHub Releases (see [Connecting to Zed](#connecting-to-zed)); nothing else is installed.

Documentation: [Zed extensions](https://zed.dev/docs/extensions/developing-extensions),
[DeepL API](https://developers.deepl.com/docs), [MyMemory API](https://mymemory.translated.net/doc/spec.php).
