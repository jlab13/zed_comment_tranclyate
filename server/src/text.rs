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

pub fn clean(raw: &str) -> String {
    if let Some(text) = raw.strip_prefix("//") {
        let text = text.trim_start_matches(['/', '!']);
        // Прибираємо один пробіл після маркера, зберігаючи відступи вкладених списків.
        return text.strip_prefix(' ').unwrap_or(text).trim_end().to_owned();
    }
    let text = raw.strip_prefix("/*").unwrap_or(raw);
    let text = text.strip_suffix("*/").unwrap_or(text);
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

// З’єднуємо перенесений текст, але зберігаємо абзаци, пункти списків і Doxygen-теги.
pub fn paragraphs(text: &str) -> String {
    let mut result = String::new();
    let mut paragraph_break = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            paragraph_break = !result.is_empty();
            continue;
        }
        let structured = structured_line(trimmed);
        if !result.is_empty() {
            result.push_str(if paragraph_break {
                "\n\n"
            } else if structured {
                "\n"
            } else {
                " "
            });
        }
        result.push_str(if structured { line.trim_end() } else { trimmed });
        paragraph_break = false;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
