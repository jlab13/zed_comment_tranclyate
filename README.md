**English** · [Українська](README_UK.md)

# Code Comment Translation for Zed

An extension for **macOS and Linux** that shows a translation of the comment under the cursor
in Zed's standard hover popup, in the system language (or the language set in `target_language`).
The default provider on both macOS and Linux is **Google Translate**, which needs no API key;
when Google does not answer, it falls back to **MyMemory**.
**Apple Translation** is available on macOS 26+ through configuration.
Supports **C, C++, Rust, Python, Shell Script (Bash), Swift, and Dart**: line, block,
and documentation comments, plus Python docstrings. Tree-sitter tells strings, raw strings,
interpolation, and heredocs apart from comments. The code in the file is never modified.

| Zed language | Comments | Notes |
| --- | --- | --- |
| C, C++ | `//`, `///`, `//!`, `/* */`, `/** */`, `/*! */` | `.h` works in both C and C++ mode |
| Rust | `//`, `///`, `//!`, `/* */`, `/** */`, `/*! */` | nested block comments |
| Swift | `//`, `///`, `/* */`, `/** */` | nested block comments; the language comes from the Swift extension |
| Dart | `//`, `///`, `/* */`, `/** */` | nested block comments; the language comes from the Dart extension |
| Python | `#`, docstrings | a `#!` shebang on the first line is not translated |
| Shell Script | `#` | a `#!` shebang on the first line is not translated |

## Architecture

```text
Zed → WASM extension → Rust LSP → Google Translate (Rust, all platforms)
                             ↳ MyMemory (Rust, fallback for Google)
                             → Apple Translation (Swift, macOS only)
```

- `src/lib.rs` — the Zed extension that launches the server.
- `server/src/` — the LSP, comment detection, UTF-16 positions, and the cache.
- `server/src/languages.rs` — the language table: languageId, file extensions, grammar, and markers.
- `server/src/segments.rs` — code protection: splitting into prose and code blocks, labels for inline code.
- `server/src/locale.rs` — resolves the target language from the system locale.
- `server/src/google.rs` — the shared Google HTTP client built on reqwest/rustls.
- `server/src/mymemory.rs` — the MyMemory client: request chunking, quota and error handling.
- `native/` — the Swift Apple translator, strategy selection, and the C-compatible bridge.
- `server/build.rs` — links Swift only on macOS with the `apple` feature.
- `examples/` — Zed settings, a key binding, and demo files with English comments for every supported language.
- `scripts/` — build and test scripts.

For Apple, the LSP calls the Swift library in the same process through the C ABI. The response
arrives through an asynchronous callback. Swift copies the input string and Rust copies the
response before the callback returns, so memory ownership never crosses the language boundary.
In the Apple build the main thread services the macOS queue, while Tokio/LSP runs on another
thread of the same process. On Linux, Tokio/LSP runs directly without Swift or Apple frameworks.
The target language comes from the system locale or from `target_language` (see
[Translation language](#translation-language)). The source language is set with `source_language`;
`auto` uses NaturalLanguage for Apple and Google's language detection for Google.
If the source language matches the target language, the text is returned unchanged.
On macOS 26.4+, Apple checks `highFidelity` first and then `lowLatency`; translation runs
locally. Google uses the portable Rust reqwest HTTP client with rustls TLS and needs internet access.
Apple never switches to another provider. Google falls back to MyMemory unless
`"fallback": "none"` is set (see [MyMemory](#mymemory)).
Up to 256 successful results are cached in memory; when the cache is full, it is cleared.

## Building

Both platforms need Rust via rustup, a C compiler for Tree-sitter, and access to crates.io
for the first dependency download. On Debian/Ubuntu, install a C compiler with
`sudo apt install build-essential`. Google does not need OpenSSL.

On macOS, the Apple build additionally requires macOS 26+ and Xcode with the macOS 26.4+ SDK.
Linux does not use Swift, Xcode, or the Apple SDK.

```sh
rustup target add wasm32-wasip2
./scripts/build.sh
```

The script builds `target/release/comment-translate-lsp` and the WASM extension. On macOS, Cargo
embeds the Swift library automatically; on Linux, `build.rs` skips that step.
`cfg(all(target_os = "macos", feature = "apple"))` isolates every Apple call.

To build without Apple, even on macOS:

```sh
cargo build --release -p comment-translate-lsp --no-default-features
```

In this mode the default provider is Google, `xcrun` is never invoked, and selecting `apple`
returns a clear error. On Linux, the `apple` feature adds no Apple code even though it is
enabled by default. Do not build the whole workspace for WASM: the LSP is a native process.
If you previously set `initialization_options.helper_path`, remove it.

## Choosing a provider

Without a `provider` field, **Google is used on all platforms**. For Google, add to your Zed settings:

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

To switch back to Apple on macOS, set `"provider": "apple"`.
The `target_language` field is described in [Translation language](#translation-language).
On Linux, an explicit `apple` selection is rejected without contacting Google.
Keep your existing `binary` field and other settings. After switching, restart the language
servers or Zed. Examples: [`zed-settings.json`](examples/zed-settings.json)
and [`zed-settings-google.json`](examples/zed-settings-google.json). For Linux there is
[`zed-settings-linux.json`](examples/zed-settings-linux.json): replace the server path.

Google works without API keys or Apple language packs, but it needs internet access and sends
the comment text to Google. It uses the web endpoint
`https://translate.googleapis.com/translate_a/single` with `client=gtx`, not the paid Cloud
Translation API. This web interface may change or rate-limit requests.
HTTP 403/429, network errors, and malformed JSON are shown in the hover in Ukrainian.
Caching and joining lines into paragraphs work for every provider.

Checking Google from the CLI:

```sh
echo '{"provider":"google","text":"Return the next counter value.","source_language":"en"}' \
  | ./target/release/comment-translate-lsp --translate
```

`--check en` returns `network_required` for the default Google provider; verify access to Google
with a real translation through `--translate` with `"provider":"google"`.

### MyMemory

[MyMemory](https://mymemory.translated.net/doc/spec.php) is a free public translation API by
Translated S.r.l. (Italy). It needs no key and is called openly as
`comment-translate-lsp/<version>`. It serves as Google's fallback by default: when Google cannot
be reached, rate-limits requests, or returns an error, the same paragraph goes to MyMemory, and
the reason is written to the language server log. After HTTP 429 or 403 from Google, the server
skips Google for 5 minutes and sends requests straight to MyMemory instead of hitting a service
that is already refusing them. When fallback is enabled, Google gets half of `timeout_seconds`
and MyMemory the other half. If both fail, the hover shows both reasons. Apple never falls back.

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
The hover header shows the language code: `Переклад → uk`. Error messages stay in Ukrainian.

## Apple translation models (macOS only)

On macOS 26.4+, check that Apple Intelligence is enabled and its models are downloaded:
`highFidelity` can work without separate language packs. Apple may fall back to traditional
models; the strategy name does not guarantee a specific model. If no strategy is ready,
download the source and target languages in **System Settings → General → Language & Region →
Translation Languages**. Availability is checked for the specific language pair.
macOS 26.0–26.3 uses regular language packs.
Checking English → Ukrainian:

```sh
echo '{"provider":"apple","operation":"check","source_language":"en","target_language":"uk"}' \
  | ./target/release/comment-translate-lsp --translate
```

A `"status":"installed"` response means it is ready. The `strategy` field shows the selected
strategy (`highFidelity` or `lowLatency`), and `checked_strategies` shows the results of the
checks that ran. A ready `highFidelity` is used right away without checking `lowLatency`.
`supported` means the required models are not ready yet; `unsupported` means the pair is not supported.
The check is limited to 15 seconds.
The LSP never shows download dialogs. If packs are missing, the hover explains
what to do. Packs are downloaded through the system interface.

Checking a translation through the embedded library:

```sh
echo '{"provider":"apple","text":"Return the next counter value.","source_language":"en"}' \
  | ./target/release/comment-translate-lsp --translate
```

## Connecting to Zed

1. Run `./scripts/build.sh`.
2. In the Zed command palette, run `zed: install dev extension` and select the project root.
3. Add the fields you need from [`examples/zed-settings.json`](examples/zed-settings.json)
   to your personal Zed settings. Check the absolute path to the LSP.
4. After changing `initialization_options`, restart the language servers or Zed.
5. Open a file in a supported language, hover over a comment or place the cursor inside
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
./scripts/test_native.sh
```

The smoke test uses the real LSP and the static Swift bridge with Ukrainian text,
which needs no language models. It covers `.c`, `.h` in C and C++ modes, Rust, Python,
Shell Script, ignoring unsupported languages, hover, editing, closing,
limits, and process shutdown. Per-language Rust tests cover comment node kinds,
markers inside strings, line grouping, docstrings, shebangs, and UTF-16 positions. Separate Rust
tests cover caching, errors, timeouts, Unicode transfer, and safely ignoring a late callback
after cancellation. `test_native.sh` checks Swift and the CLI only for the Apple build on macOS.
Shared Rust tests cover Google: parameter encoding, JSON, and HTTP errors. Two loopback HTTP
tests are marked `ignored` because they need permission to open local TCP sockets:

```sh
cargo test -p comment-translate-lsp -- --include-ignored
cargo test -p comment-translate-lsp --no-default-features
```

Linux CI runs these checks, builds the server and WASM, and checks the CLI and LSP.
Regular tests need neither internet access nor language models. Real translation needs
ready Apple models or access to Google.

## Code protection

The translator receives only prose; code never reaches it.

- **Code blocks** — Markdown fences (```` ``` ````, `~~~`), Doxygen `@code … @endcode`,
  `\code … \endcode`, and `@verbatim … @endverbatim`, and Python doctest examples (`>>>`
  up to the next blank line) — are shown verbatim and never sent for translation. The prose
  paragraphs around them are translated and cached separately.
- **Code inside sentences** is replaced with `x0_`, `x1_`, … labels before translation and put
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

C, C++, Rust, Python, Shell Script, Swift, and Dart are supported on macOS and Linux;
Apple translation requires macOS 26+. Other languages are ignored: no hover appears for them.
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
limited by `timeout_seconds`. On timeout, the Google request is cancelled in Rust; for Apple,
Rust detaches the callback and cancels the Swift task and the active session. System calls may
not respond to cancellation immediately, so the Apple bridge holds at most 8 unfinished tasks
and rejects new ones until a slot frees up. A failure in the native library now affects the LSP
process, because the translator runs inside it.
There is no dedicated translation command: Zed's standard hover is used, so translation can
also be triggered by hovering the mouse. Native programs are not installed automatically.

Documentation: [Zed extensions](https://zed.dev/docs/extensions/developing-extensions),
[Apple Translation](https://developer.apple.com/documentation/translation),
[session for installed languages](https://developer.apple.com/documentation/translation/translationsession/init(installedsource:target:)).
