use crate::languages::LanguageSpec;

// LSP використовує UTF-16, а Tree-sitter — байтові зміщення UTF-8.
pub fn byte_offset(source: &str, line_number: u32, character: u32) -> Option<usize> {
    let mut start = 0;
    for _ in 0..line_number {
        start += source.get(start..)?.find('\n')? + 1;
    }
    let line = source
        .get(start..)?
        .split('\n')
        .next()?
        .trim_end_matches('\r');
    let mut units = 0;
    for (offset, ch) in line.char_indices() {
        if units == character {
            return Some(start + offset);
        }
        units += ch.len_utf16() as u32;
    }
    (units == character).then_some(start + line.len())
}

pub fn position(source: &str, offset: usize) -> (u32, u32) {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|&byte| byte == b'\n').count() as u32;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count() as u32;
    (line, column)
}

/// Прибирає маркери коментаря відповідної мови та спільний відступ.
pub fn clean(raw: &str, language: &LanguageSpec) -> String {
    if let Some(text) = language
        .line_markers
        .iter()
        .find_map(|marker| strip_line_marker(raw, marker))
    {
        return text.to_owned();
    }
    let text = match language.block {
        Some((open, close)) if raw.starts_with(open) => {
            let text = &raw[open.len()..];
            let text = text.strip_suffix(close).unwrap_or(text);
            // `/*!` — внутрішній документаційний коментар Rust або Doxygen у стилі Qt.
            // Вкладені блоки Rust, Swift і Dart зберігають внутрішні маркери як частину тексту.
            text.strip_prefix('!').unwrap_or(text)
        }
        _ => raw,
    };
    let lines: Vec<_> = text
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let trimmed = line.trim_start();
            let content = if let Some(content) = trimmed.strip_prefix('*') {
                content.strip_prefix(' ').unwrap_or(content)
            } else if index == 0 {
                trimmed
            } else {
                line
            };
            content.trim_end()
        })
        .collect();
    let margin = lines
        .iter()
        .filter(|line| !line.is_empty())
        .map(|line| indentation(line))
        .min()
        .unwrap_or(0);
    lines
        .into_iter()
        .map(|line| {
            if line.is_empty() {
                line
            } else {
                &line[margin..]
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_owned()
}

fn strip_line_marker<'a>(raw: &'a str, marker: &str) -> Option<&'a str> {
    let text = raw.strip_prefix(marker)?;
    // Повтори маркера (`///`, `##`) і `!` у `//!` теж належать до маркера.
    let text =
        text.trim_start_matches(|ch: char| marker.contains(ch) || (ch == '!' && marker == "//"));
    // Прибираємо один пробіл після маркера, зберігаючи відступи вкладених списків.
    Some(text.strip_prefix(' ').unwrap_or(text).trim_end())
}

/// Повертає вміст рядкового літерала Python без префікса (`r`, `u`) і лапок.
pub fn unquote(raw: &str) -> &str {
    let body = raw.trim_start_matches(|ch: char| ch.is_ascii_alphabetic());
    for quotes in ["\"\"\"", "'''", "\"", "'"] {
        if let Some(inner) = body.strip_prefix(quotes) {
            return inner.strip_suffix(quotes).unwrap_or(inner);
        }
    }
    body
}

/// Вирівнює docstring так само, як `inspect.cleandoc` у Python.
pub fn cleandoc(text: &str) -> String {
    let lines: Vec<_> = text.lines().map(expand_tabs).collect();
    let spaces = |line: &str| line.len() - line.trim_start_matches(' ').len();
    // Відступ першого рядка не враховується: він іде одразу після лапок.
    let margin = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| spaces(line))
        .min()
        .unwrap_or(0);
    let lines: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if line.trim().is_empty() {
                ""
            } else if index == 0 {
                line.trim()
            } else {
                line[margin..].trim_end()
            }
        })
        .collect();
    let first = lines.iter().position(|line| !line.is_empty());
    let last = lines.iter().rposition(|line| !line.is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    }
}

fn expand_tabs(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut column = 0;
    for ch in line.chars() {
        if ch == '\t' {
            let width = 8 - column % 8;
            result.extend(std::iter::repeat_n(' ', width));
            column += width;
        } else {
            result.push(ch);
            column += 1;
        }
    }
    result
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn structured_line(line: &str) -> bool {
    let after_marker = |tail: &str| tail.is_empty() || tail.starts_with(char::is_whitespace);
    if let Some(tail) = line.strip_prefix(['-', '*', '+', '•']) {
        return after_marker(tail);
    }
    if let Some(tail) = line.strip_prefix(['@', '\\']) {
        return tail.starts_with(|ch: char| ch.is_ascii_alphabetic());
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0
        && line[digits..]
            .strip_prefix(['.', ')'])
            .is_some_and(after_marker)
}

// Заголовок Markdown: від одного до шести `#`, після яких іде пробіл або кінець рядка.
fn heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|&byte| byte == b'#').count();
    (1..=6).contains(&hashes)
        && line[hashes..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

// Огородження блоку коду Markdown (```` ``` ```` або `~~~`).
fn code_fence(line: &str) -> Option<&'static str> {
    ["```", "~~~"]
        .into_iter()
        .find(|marker| line.starts_with(marker))
}

// З’єднуємо перенесений текст, але зберігаємо абзаци, пункти списків, Doxygen-теги,
// заголовки Markdown і блоки коду.
pub fn paragraphs(text: &str) -> String {
    let mut result = String::new();
    let mut paragraph_break = false;
    // Після заголовка чи блоку коду наступний рядок не приєднується до попереднього.
    let mut line_break = false;
    let mut fence = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(marker) = fence {
            // Код усередині огородження залишаємо дослівно, разом із порожніми рядками.
            result.push('\n');
            result.push_str(line.trim_end());
            if trimmed.starts_with(marker) {
                fence = None;
                line_break = true;
            }
            continue;
        }
        if trimmed.is_empty() {
            paragraph_break = !result.is_empty();
            continue;
        }
        fence = code_fence(trimmed);
        let standalone = fence.is_some() || heading(trimmed);
        let structured = standalone || structured_line(trimmed);
        if !result.is_empty() {
            result.push_str(if paragraph_break {
                "\n\n"
            } else if structured || line_break {
                "\n"
            } else {
                " "
            });
        }
        result.push_str(if structured { line.trim_end() } else { trimmed });
        paragraph_break = false;
        line_break = standalone;
    }
    // Незакритий блок коду може закінчуватися порожніми рядками.
    result.truncate(result.trim_end().len());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::{BASH, C, PYTHON, RUST, SWIFT};

    /// Наявні тести перевіряють C, тому очищення за замовчуванням використовує правила C.
    fn clean(raw: &str) -> String {
        super::clean(raw, &C)
    }

    #[test]
    fn utf16_surrogate_pair_is_not_split() {
        assert_eq!(byte_offset("// 😀 value", 0, 4), None);
        assert_eq!(byte_offset("// 😀 value", 0, 5), Some(7));
    }

    #[test]
    fn coordinates_round_trip_with_unicode_and_crlf() {
        let source = "// 😀\r\n/* Текст */\n";
        for (offset, ch) in source.char_indices() {
            if ch == '\r' || ch == '\n' {
                continue;
            }
            let (line, column) = position(source, offset);
            assert_eq!(byte_offset(source, line, column), Some(offset));
        }
    }

    #[test]
    fn rejects_out_of_bounds_positions() {
        assert_eq!(byte_offset("a", 1, 0), None);
        assert_eq!(byte_offset("a", 0, 2), None);
        assert_eq!(byte_offset("", 0, 0), Some(0));
        assert_eq!(byte_offset("a\n", 1, 0), Some(2));
    }

    #[test]
    fn strips_comment_delimiters() {
        assert_eq!(clean("// Return value"), "Return value");
        assert_eq!(clean("/// Return value"), "Return value");
        assert_eq!(clean("//! Return value"), "Return value");
        assert_eq!(clean("/* Return value */"), "Return value");
    }

    #[test]
    fn preserves_multiline_content_and_identifiers() {
        assert_eq!(
            clean("/**\r\n * Return `get_value()`.\r\n *\r\n * Keep ownership.\r\n */"),
            "Return `get_value()`.\n\nKeep ownership."
        );
    }

    #[test]
    fn empty_documentation_has_no_text() {
        assert_eq!(clean("/** */"), "");
        assert_eq!(clean("//"), "");
    }
    #[test]
    fn joins_wrapped_text_into_one_paragraph() {
        assert_eq!(paragraphs("  This structure represents a Cyphal node,\n  containing transport-specific context.  "),
            "This structure represents a Cyphal node, containing transport-specific context.");
    }

    #[test]
    fn preserves_paragraph_boundaries_and_handles_crlf() {
        assert_eq!(
            paragraphs("\r\n First line\r\ncontinues.\r\n  \r\n\r\nSecond paragraph.\r\n"),
            "First line continues.\n\nSecond paragraph."
        );
        assert_eq!(paragraphs(" \n\r\n"), "");
    }

    #[test]
    fn preserves_numbered_items_and_joins_their_wrapped_descriptions() {
        let text = "Initialize the transport\nby:\n1. Open the interface\n   and check its state.\n2. Initialize the instance\n3) Set the node ID";
        assert_eq!(paragraphs(text), "Initialize the transport by:\n1. Open the interface and check its state.\n2. Initialize the instance\n3) Set the node ID");
    }

    #[test]
    fn preserves_bullets_and_nested_indentation() {
        let text = "Options:\n- First\n  * Nested\n    continuation\n+ Second\n• Third";
        assert_eq!(
            paragraphs(text),
            "Options:\n- First\n  * Nested continuation\n+ Second\n• Third"
        );
    }

    #[test]
    fn preserves_doxygen_commands_and_wrapped_parameter_descriptions() {
        let text = "@brief Initialize a node.\n\n@param[in] node Pointer to\n  the node.\n@param node_id Node ID.\n@return int 0 on success.\n\\note Keep the node alive.";
        assert_eq!(paragraphs(text), "@brief Initialize a node.\n\n@param[in] node Pointer to the node.\n@param node_id Node ID.\n@return int 0 on success.\n\\note Keep the node alive.");
    }

    #[test]
    fn decimal_numbers_and_negative_values_are_not_list_markers() {
        assert_eq!(
            paragraphs("Rate:\n1.25 MHz\nwith offset\n-127 and\n+127 limits."),
            "Rate: 1.25 MHz with offset -127 and +127 limits."
        );
    }

    #[test]
    fn cleaning_keeps_nested_list_indentation_for_both_comment_styles() {
        let block = "/**\n * - First\n *   - Nested\n *     continuation\n * * Second\n */";
        assert_eq!(
            paragraphs(&clean(block)),
            "- First\n  - Nested continuation\n* Second"
        );
        let lines = [
            "// - First",
            "//   - Nested",
            "//     continuation",
            "// * Second",
        ];
        let cleaned = lines.into_iter().map(clean).collect::<Vec<_>>().join("\n");
        assert_eq!(paragraphs(&cleaned), paragraphs(&clean(block)));
    }

    #[test]
    fn strips_rust_and_doxygen_documentation_markers() {
        for raw in [
            "/// Outer",
            "//! Inner",
            "/** Outer */",
            "/*! Inner */",
            "/*!\n * Inner\n */",
        ] {
            let expected = if raw.contains("Outer") {
                "Outer"
            } else {
                "Inner"
            };
            assert_eq!(super::clean(raw, &RUST), expected, "{raw}");
        }
        assert_eq!(super::clean("/*! Qt style */", &C), "Qt style");
    }

    #[test]
    fn nested_block_comments_keep_inner_markers_as_text() {
        let raw = "/* outer /* inner */ tail */";
        assert_eq!(super::clean(raw, &RUST), "outer /* inner */ tail");
        assert_eq!(super::clean(raw, &SWIFT), "outer /* inner */ tail");
    }

    #[test]
    fn strips_hash_markers_for_python_and_bash() {
        for language in [&PYTHON, &BASH] {
            assert_eq!(super::clean("# Return value", language), "Return value");
            assert_eq!(super::clean("## Section", language), "Section");
            assert_eq!(super::clean("#   - nested", language), "  - nested");
            assert_eq!(super::clean("#", language), "");
            assert_eq!(super::clean("##########", language), "");
        }
    }

    #[test]
    fn unquote_strips_python_prefixes_and_quotes() {
        assert_eq!(unquote(r#""""Doc.""""#), "Doc.");
        assert_eq!(unquote("r'''Doc.'''"), "Doc.");
        assert_eq!(unquote("U\"Doc.\""), "Doc.");
        assert_eq!(unquote("'Doc.'"), "Doc.");
        assert_eq!(unquote("\"\""), "");
        assert_eq!(unquote("\"\"\"\"\"\""), "");
        assert_eq!(unquote("\"\"\"unterminated"), "unterminated");
    }

    #[test]
    fn cleandoc_matches_python_inspect() {
        // Очікування отримано з inspect.cleandoc у Python 3.
        assert_eq!(
            cleandoc("\n    Summary.\n\n        Indented.\n    End.\n    "),
            "Summary.\n\n    Indented.\nEnd."
        );
        assert_eq!(
            cleandoc("First line.\n        Second.\n          Third."),
            "First line.\nSecond.\n  Third."
        );
        assert_eq!(
            cleandoc("\tTab\n\tindented\n\t\tmore"),
            "Tab\nindented\n        more"
        );
        assert_eq!(cleandoc("Один рядок 😀"), "Один рядок 😀");
        // На відміну від Python, рядки з самих пробілів вважаються порожніми: hover не потрібен.
        assert_eq!(cleandoc("  \n  \n"), "");
    }

    #[test]
    fn fenced_code_blocks_are_kept_verbatim() {
        let text = "Example:\n\n```rust\nlet x = 1;\n\n    nested();\n```\nAfter the\ncode.";
        assert_eq!(
            paragraphs(text),
            "Example:\n\n```rust\nlet x = 1;\n\n    nested();\n```\nAfter the code."
        );
        assert_eq!(
            paragraphs("Text\n~~~\n- not a list\n~~~"),
            "Text\n~~~\n- not a list\n~~~"
        );
        assert_eq!(
            paragraphs("Open\n```\ncode\n\n"),
            "Open\n```\ncode",
            "незакритий блок не залишає порожніх рядків у кінці"
        );
    }

    #[test]
    fn markdown_headings_stay_on_separate_lines() {
        let text = "# Examples\nCall the\nfunction.\n## Panics\nNever\n#hashtag inline.\n#";
        assert_eq!(
            paragraphs(text),
            "# Examples\nCall the function.\n## Panics\nNever #hashtag inline.\n#"
        );
        assert_eq!(
            paragraphs("Text\n####### seven"),
            "Text ####### seven",
            "більше шести # — не заголовок"
        );
    }
}
