use crate::languages::LanguageSpec;
use crate::text::{byte_offset, clean, cleandoc, paragraphs, position, unquote};
use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::Node;

pub struct Comment {
    pub text: String,
    pub range: Range,
}

pub fn at(source: &str, cursor: Position, language: &LanguageSpec) -> Option<Comment> {
    let offset = byte_offset(source, cursor.line, cursor.character)?;
    if offset >= source.len() {
        return None;
    }
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&(language.grammar)()).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    let node = comment_node(source, root, offset, language)?;
    if offset >= comment_end(source, node) {
        return None;
    }
    let (first, last, text) = if is_comment(source, node, language) {
        comment_group(source, root, node, language)
    } else {
        (node, node, docstring_text(source, node))
    };
    let text = paragraphs(&text);
    if text.is_empty() {
        return None;
    }
    let (start_line, start_column) = position(source, first.start_byte());
    let (end_line, end_column) = position(source, comment_end(source, last));
    Some(Comment {
        text,
        range: Range::new(
            Position::new(start_line, start_column),
            Position::new(end_line, end_column),
        ),
    })
}

// Повертає перший і останній коментар групи та очищений текст усіх рядків.
fn comment_group<'tree>(
    source: &str,
    root: Node<'tree>,
    node: Node<'tree>,
    language: &LanguageSpec,
) -> (Node<'tree>, Node<'tree>, String) {
    let mut first = node;
    let mut last = node;
    let mut parts = std::collections::VecDeque::from([clean(comment_text(source, node), language)]);
    if standalone_line_comment(source, node, language) {
        // Об’єднуємо лише сусідні рядки коментарів без коду перед маркером.
        loop {
            let line_start = source[..first.start_byte()]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            if line_start == 0 {
                break;
            }
            let previous_start = source[..line_start - 1]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            let Some(previous) = comment_on_line(source, root, previous_start, language) else {
                break;
            };
            if comment_end(source, previous) > first.start_byte() {
                break;
            }
            parts.push_front(clean(comment_text(source, previous), language));
            first = previous;
        }
        while let Some((suffix, _)) = source[comment_end(source, last)..].split_once('\n') {
            if !suffix.trim().is_empty() {
                break;
            }
            let next_start = comment_end(source, last) + suffix.len() + 1;
            let Some(next) = comment_on_line(source, root, next_start, language) else {
                break;
            };
            parts.push_back(clean(comment_text(source, next), language));
            last = next;
        }
    }
    let text = parts.into_iter().collect::<Vec<_>>().join("\n");
    (first, last, text)
}

fn comment_end(source: &str, node: Node<'_>) -> usize {
    // Tree-sitter включає CR у кінець // (а Rust — увесь перенос після ///),
    // але LSP-координата закінчується до переносу рядка.
    node.start_byte()
        + source[node.byte_range()]
            .trim_end_matches(['\r', '\n'])
            .len()
}

fn comment_text<'source>(source: &'source str, node: Node<'_>) -> &'source str {
    &source[node.start_byte()..comment_end(source, node)]
}

// Знаходить коментар або docstring, що містить байт `offset`.
fn comment_node<'tree>(
    source: &str,
    root: Node<'tree>,
    offset: usize,
    language: &LanguageSpec,
) -> Option<Node<'tree>> {
    let mut node = root.descendant_for_byte_range(offset, offset + 1)?;
    while !is_comment(source, node, language)
        && !(language.docstrings && is_docstring(source, node))
    {
        node = node.parent()?;
    }
    (offset < node.end_byte()).then_some(node)
}

fn is_comment(source: &str, node: Node<'_>, language: &LanguageSpec) -> bool {
    // Shebang має синтаксис коментаря, але є інструкцією для запуску скрипта.
    let shebang = language.shebang && node.start_byte() == 0 && source.starts_with("#!");
    language.comment_kinds.contains(&node.kind()) && !shebang
}

// Docstring — рядковий літерал, що є першою інструкцією модуля, класу або функції Python.
fn is_docstring(source: &str, node: Node<'_>) -> bool {
    let parts: Vec<_> = match node.kind() {
        "string" => vec![node],
        "concatenated_string" => string_parts(node).collect(),
        _ => return false,
    };
    // f-рядки, байтові й шаблонні літерали не стають `__doc__`.
    let plain = parts.iter().all(|part| {
        source[part.byte_range()]
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .all(|prefix| matches!(prefix, 'r' | 'R' | 'u' | 'U'))
    });
    let Some(statement) = node.parent() else {
        return false;
    };
    let Some(body) = statement.parent() else {
        return false;
    };
    let owner = match body.kind() {
        "module" => true,
        "block" => body.parent().is_some_and(|definition| {
            matches!(
                definition.kind(),
                "function_definition" | "class_definition"
            )
        }),
        _ => false,
    };
    let mut cursor = body.walk();
    let first_statement = body
        .named_children(&mut cursor)
        .find(|child| child.kind() != "comment");
    plain
        && owner
        && statement.kind() == "expression_statement"
        && statement.named_child_count() == 1
        && first_statement == Some(statement)
}

fn string_parts(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    (0..node.named_child_count())
        .filter_map(move |index| node.named_child(index))
        .filter(|child| child.kind() == "string")
}

fn docstring_text(source: &str, node: Node<'_>) -> String {
    let body: String = if node.kind() == "string" {
        unquote(&source[node.byte_range()]).to_owned()
    } else {
        string_parts(node)
            .map(|part| unquote(&source[part.byte_range()]))
            .collect()
    };
    cleandoc(&body)
}

fn standalone_line_comment(source: &str, node: Node<'_>, language: &LanguageSpec) -> bool {
    let line_start = source[..node.start_byte()]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let text = &source[node.byte_range()];
    is_comment(source, node, language)
        && language
            .line_markers
            .iter()
            .any(|marker| text.starts_with(marker))
        && source[line_start..node.start_byte()].trim().is_empty()
}

fn comment_on_line<'tree>(
    source: &str,
    root: Node<'tree>,
    start: usize,
    language: &LanguageSpec,
) -> Option<Node<'tree>> {
    let line = source.get(start..)?.split('\n').next()?;
    let trimmed = line.trim_start();
    if !language
        .line_markers
        .iter()
        .any(|marker| trimmed.starts_with(marker))
    {
        return None;
    }
    let offset = start + line.len() - trimmed.len();
    let node = comment_node(source, root, offset, language)?;
    // Перевіряємо дерево, щоб не захопити маркер усередині рядка, heredoc або /* ... */.
    (node.start_byte() == offset && standalone_line_comment(source, node, language)).then_some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::{ALL, BASH, C, CPP, DART, PYTHON, RUST, SWIFT};

    /// Наявні тести перевіряють C, тому пошук за замовчуванням використовує правила C.
    fn at(source: &str, cursor: Position) -> Option<Comment> {
        super::at(source, cursor, &C)
    }

    fn text_in(language: &LanguageSpec, source: &str, line: u32, column: u32) -> Option<String> {
        super::at(source, Position::new(line, column), language).map(|comment| comment.text)
    }

    fn range_in(language: &LanguageSpec, source: &str, line: u32, column: u32) -> Range {
        super::at(source, Position::new(line, column), language)
            .unwrap()
            .range
    }

    fn range(start: (u32, u32), end: (u32, u32)) -> Range {
        Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1))
    }

    #[test]
    fn detects_line_comment_and_excludes_end() {
        let source = "int x; // Return value\n";
        assert_eq!(
            at(source, Position::new(0, 10)).unwrap().text,
            "Return value"
        );
        assert!(at(source, Position::new(0, 22)).is_none());
        assert!(at(source, Position::new(0, 1)).is_none());
    }

    #[test]
    fn cleans_multiline_documentation() {
        let source = "/**\n * Return the value.\n * Keep ownership.\n */\nint x;";
        let comment = at(source, Position::new(2, 5)).unwrap();
        assert_eq!(comment.text, "Return the value. Keep ownership.");
        assert_eq!(
            comment.range,
            Range::new(Position::new(0, 0), Position::new(3, 3))
        );
    }

    #[test]
    fn ignores_comment_markers_inside_strings() {
        let source = "const char *s = \"// fake /* comment */\"; // Real";
        assert!(at(source, Position::new(0, 20)).is_none());
        assert_eq!(at(source, Position::new(0, 43)).unwrap().text, "Real");
    }

    #[test]
    fn handles_unicode_crlf_and_invalid_positions() {
        let source = "// 😀 value\r\n/* Текст */";
        assert_eq!(byte_offset(source, 0, 6), Some(8));
        assert_eq!(byte_offset(source, 0, 4), None);
        assert!(byte_offset(source, 3, 0).is_none());
        assert_eq!(at(source, Position::new(1, 4)).unwrap().text, "Текст");
    }

    #[test]
    fn ignores_empty_comments() {
        assert!(at("/* */", Position::new(0, 2)).is_none());
    }

    #[test]
    fn joins_line_comments_from_every_line_in_the_block() {
        let source = "// This structure represents a Cyphal node in the network, containing\n// both transport-independent information and transport-specific context data in a union.";
        let expected = "This structure represents a Cyphal node in the network, containing both transport-independent information and transport-specific context data in a union.";
        for line in 0..2 {
            let comment = at(source, Position::new(line, 5)).unwrap();
            assert_eq!(comment.text, expected);
            assert_eq!(
                comment.range,
                Range::new(Position::new(0, 0), Position::new(1, 89))
            );
        }
    }

    #[test]
    fn joins_indented_unicode_comments_and_preserves_empty_comment_lines() {
        let source = "void f(void) {\r\n    /// Перший 😀\r\n    //\r\n\t//! Другий\r\n}\r\n";
        for (line, column) in [(1, 10), (2, 5), (3, 7)] {
            let comment = at(source, Position::new(line, column)).unwrap();
            assert_eq!(comment.text, "Перший 😀\n\nДругий");
            assert_eq!(
                comment.range,
                Range::new(Position::new(1, 4), Position::new(3, 11))
            );
        }
    }

    #[test]
    fn blank_lines_code_and_block_comments_separate_groups() {
        for separator in ["\n", "int value;\n", "/* Separate block */\n"] {
            let source = format!("// First\n{separator}// Second\n// Third");
            assert_eq!(at(&source, Position::new(0, 4)).unwrap().text, "First");
            assert_eq!(
                at(&source, Position::new(2, 4)).unwrap().text,
                "Second Third"
            );
        }
    }

    #[test]
    fn does_not_merge_comments_attached_to_different_code_lines() {
        let source = "int a; // First\nint b; // Second\n// Standalone\n// Continuation";
        assert_eq!(at(source, Position::new(0, 10)).unwrap().text, "First");
        assert_eq!(at(source, Position::new(1, 10)).unwrap().text, "Second");
        assert_eq!(
            at(source, Position::new(2, 5)).unwrap().text,
            "Standalone Continuation"
        );
    }

    #[test]
    fn does_not_merge_comment_markers_from_multiline_block_comments() {
        let source = "/*\n// Inside block\n*/\n// Outside\n// Continuation";
        assert_eq!(
            at(source, Position::new(3, 5)).unwrap().text,
            "Outside Continuation"
        );
        assert_eq!(
            at(source, Position::new(1, 5)).unwrap().text,
            "// Inside block"
        );
    }
    #[test]
    fn line_and_block_comments_produce_identical_paragraphs() {
        let line = "// Перший рядок\n// продовжується.\n//\n// Другий абзац.";
        let block = "/**\n * Перший рядок\n * продовжується.\n *\n * Другий абзац.\n */";
        let expected = "Перший рядок продовжується.\n\nДругий абзац.";
        assert_eq!(at(line, Position::new(1, 5)).unwrap().text, expected);
        assert_eq!(at(block, Position::new(2, 5)).unwrap().text, expected);
    }

    #[test]
    fn preserves_cyphal_doxygen_list_and_parameter_lines() {
        let source = r#"/**
 * @brief Initialize a CAN transport context for a Cyphal node.
 *
 * This function initializes the CAN transport for a Cyphal node by:
 * 1. Opening a SocketCAN interface
 * 2. Initializing the Canard instance with memory allocation callbacks
 * 3. Setting up the node ID
 * 4. Creating a transmission queue
 *
 * @param node Pointer to the node structure to initialize.
 * @param node_id The Cyphal node ID to assign (0-127 for CAN).
 * @return int 0 on success, negative error code on failure.
 */"#;
        let expected = "@brief Initialize a CAN transport context for a Cyphal node.\n\nThis function initializes the CAN transport for a Cyphal node by:\n1. Opening a SocketCAN interface\n2. Initializing the Canard instance with memory allocation callbacks\n3. Setting up the node ID\n4. Creating a transmission queue\n\n@param node Pointer to the node structure to initialize.\n@param node_id The Cyphal node ID to assign (0-127 for CAN).\n@return int 0 on success, negative error code on failure.";
        for line in [1, 4, 7, 10] {
            assert_eq!(at(source, Position::new(line, 6)).unwrap().text, expected);
        }
    }

    #[test]
    fn every_language_groups_line_comments_with_utf16_ranges_without_line_breaks() {
        let width = |text: &str| text.encode_utf16().count() as u32;
        for language in ALL {
            let marker = language.line_markers[0];
            for newline in ["\n", "\r\n"] {
                let source = format!(
                    "{marker} Перший 😀{newline}{marker} другий.{newline}{newline}{marker} Окремий{newline}"
                );
                let end = width(&format!("{marker} другий."));
                for line in 0..2 {
                    let comment = super::at(&source, Position::new(line, 3), language).unwrap();
                    assert_eq!(comment.text, "Перший 😀 другий.", "{marker}");
                    assert_eq!(comment.range, range((0, 0), (1, end)), "{marker}");
                }
                let emoji = width(&format!("{marker} Перший "));
                assert!(text_in(language, &source, 0, emoji).is_some());
                assert!(text_in(language, &source, 0, emoji + 1).is_none());
                assert!(text_in(language, &source, 1, end).is_none(), "{marker}");
                assert_eq!(text_in(language, &source, 3, 3).as_deref(), Some("Окремий"));
            }
        }
    }

    #[test]
    fn cpp_line_block_and_documentation_comments() {
        let source = "/// Documented function.\nint f();\n/** Block documentation. */\nint g(); // Trailing note.\n";
        assert_eq!(
            text_in(&CPP, source, 0, 5).as_deref(),
            Some("Documented function.")
        );
        assert_eq!(
            text_in(&CPP, source, 2, 5).as_deref(),
            Some("Block documentation.")
        );
        assert_eq!(
            text_in(&CPP, source, 3, 12).as_deref(),
            Some("Trailing note.")
        );
    }

    #[test]
    fn cpp_raw_string_markers_are_not_comments() {
        let source = "auto a = R\"(// x /* y */)\";\nauto b = R\"tag(// z)tag\"; // Real\n";
        assert!(text_in(&CPP, source, 0, 14).is_none());
        assert!(text_in(&CPP, source, 0, 19).is_none());
        assert!(text_in(&CPP, source, 1, 16).is_none());
        assert_eq!(text_in(&CPP, source, 1, 30).as_deref(), Some("Real"));
    }

    #[test]
    fn rust_line_block_and_documentation_comments() {
        let source = "//! Crate documentation.\n\n/// Item documentation.\nfn f() {} // Plain note.\n/** Block documentation. */\nstruct S;\n/*! Inner block documentation. */\n";
        for (line, column, expected) in [
            (0, 5, "Crate documentation."),
            (2, 5, "Item documentation."),
            (3, 13, "Plain note."),
            (4, 5, "Block documentation."),
            (6, 5, "Inner block documentation."),
        ] {
            assert_eq!(
                text_in(&RUST, source, line, column).as_deref(),
                Some(expected)
            );
        }
        assert!(text_in(&RUST, source, 3, 5).is_none());
    }

    #[test]
    fn rust_documentation_range_ends_before_line_break() {
        for newline in ["\n", "\r\n"] {
            let source = format!("/// Перший 😀{newline}/// Другий{newline}fn f() {{}}{newline}");
            for line in 0..2 {
                let comment = super::at(&source, Position::new(line, 5), &RUST).unwrap();
                assert_eq!(comment.text, "Перший 😀 Другий");
                assert_eq!(comment.range, range((0, 0), (1, 10)));
            }
            // Граматика Rust включає перенос у вузол `///`, але курсор на ньому не в коментарі.
            assert!(text_in(&RUST, &source, 0, 13).is_none());
            assert!(text_in(&RUST, &source, 1, 10).is_none());
        }
    }

    #[test]
    fn rust_string_markers_are_not_comments() {
        let source =
            "fn f() {\n    let a = r#\"// x /* y */\"#;\n    let b = \"// z\"; // Real\n}\n";
        assert!(text_in(&RUST, source, 1, 16).is_none());
        assert!(text_in(&RUST, source, 1, 21).is_none());
        assert!(text_in(&RUST, source, 2, 14).is_none());
        assert_eq!(text_in(&RUST, source, 2, 23).as_deref(), Some("Real"));
    }

    #[test]
    fn nested_block_comments_are_single_comments() {
        for language in [&RUST, &SWIFT, &DART] {
            let source = "/* outer /* inner */ tail */\n";
            for column in [3, 12, 22] {
                assert_eq!(
                    text_in(language, source, 0, column).as_deref(),
                    Some("outer /* inner */ tail")
                );
            }
            assert_eq!(range_in(language, source, 0, 22), range((0, 0), (0, 28)));
        }
    }

    #[test]
    fn swift_line_block_and_documentation_comments() {
        let source = "/// Documented.\nfunc f() {} // Trailing.\n/** Block documentation. */\nlet x = 1\n/* Plain block. */\n";
        for (line, column, expected) in [
            (0, 5, "Documented."),
            (1, 15, "Trailing."),
            (2, 5, "Block documentation."),
            (4, 4, "Plain block."),
        ] {
            assert_eq!(
                text_in(&SWIFT, source, line, column).as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn swift_strings_and_interpolation_are_not_comments() {
        let source = "let a = \"\"\"\n    // not a comment\n    /* nope */\n    \"\"\"\nlet b = \"\\(value) // text\"\nlet c = #\"// raw\"#\n";
        assert!(text_in(&SWIFT, source, 1, 6).is_none());
        assert!(text_in(&SWIFT, source, 2, 6).is_none());
        assert!(text_in(&SWIFT, source, 4, 12).is_none());
        assert!(text_in(&SWIFT, source, 4, 19).is_none());
        assert!(text_in(&SWIFT, source, 5, 11).is_none());
    }

    #[test]
    fn dart_line_block_and_documentation_comments() {
        let source = "/// Documented.\n/// Continues.\nvoid f() {} // Trailing.\n/** Block documentation. */\nvar x = 1;\n";
        for line in 0..2 {
            assert_eq!(
                text_in(&DART, source, line, 5).as_deref(),
                Some("Documented. Continues.")
            );
            assert_eq!(range_in(&DART, source, line, 5), range((0, 0), (1, 14)));
        }
        assert_eq!(text_in(&DART, source, 2, 15).as_deref(), Some("Trailing."));
        assert_eq!(
            text_in(&DART, source, 3, 5).as_deref(),
            Some("Block documentation.")
        );
    }

    #[test]
    fn dart_interpolation_and_strings_are_not_comments() {
        let source = "var a = 'x ${b} // c';\nvar d = \"${e /* f */} g\";\nvar h = r'// i';\nvar j = '''\n// k\n''';\n";
        assert!(text_in(&DART, source, 0, 17).is_none());
        assert!(text_in(&DART, source, 0, 13).is_none());
        // Коментар у виразі інтерполяції — справжній коментар Dart.
        assert_eq!(text_in(&DART, source, 1, 16).as_deref(), Some("f"));
        assert!(text_in(&DART, source, 1, 22).is_none());
        assert!(text_in(&DART, source, 2, 11).is_none());
        assert!(text_in(&DART, source, 4, 2).is_none());
    }

    #[test]
    fn python_hash_comments_are_grouped_and_strings_are_ignored() {
        let source = "# First line\n# continues.\nvalue = \"# not comment\"  # Trailing.\nname = f\"{value} # text\"\n";
        for line in 0..2 {
            assert_eq!(
                text_in(&PYTHON, source, line, 3).as_deref(),
                Some("First line continues.")
            );
            assert_eq!(range_in(&PYTHON, source, line, 3), range((0, 0), (1, 12)));
        }
        assert!(text_in(&PYTHON, source, 2, 9).is_none());
        assert_eq!(
            text_in(&PYTHON, source, 2, 27).as_deref(),
            Some("Trailing.")
        );
        assert!(text_in(&PYTHON, source, 3, 17).is_none());
    }

    #[test]
    fn shebang_has_no_hover_and_is_not_grouped() {
        for (language, shebang) in [(&PYTHON, "#!/usr/bin/env python3"), (&BASH, "#!/bin/bash")] {
            let source = format!("{shebang}\n# Real comment.\n");
            assert!(text_in(language, &source, 0, 3).is_none());
            assert_eq!(
                text_in(language, &source, 1, 3).as_deref(),
                Some("Real comment.")
            );
            assert_eq!(range_in(language, &source, 1, 3), range((1, 0), (1, 15)));
        }
    }

    #[test]
    fn python_docstrings_of_modules_classes_and_functions() {
        let source = r#"#!/usr/bin/env python3
"""Module summary."""


class Counter:
    r'''Class summary.'''

    def next(self):
        # A comment before the docstring.
        """
        Return the next value.

            Indented details.
        """
        return 1


async def run():
    'Single quoted.'


def joined():
    "First " u"second."
"#;
        assert_eq!(
            text_in(&PYTHON, source, 1, 5).as_deref(),
            Some("Module summary.")
        );
        assert_eq!(range_in(&PYTHON, source, 1, 5), range((1, 0), (1, 21)));
        assert_eq!(
            text_in(&PYTHON, source, 5, 10).as_deref(),
            Some("Class summary.")
        );
        assert_eq!(range_in(&PYTHON, source, 5, 10), range((5, 4), (5, 25)));
        assert_eq!(
            text_in(&PYTHON, source, 8, 12).as_deref(),
            Some("A comment before the docstring.")
        );
        for (line, column) in [(9, 9), (10, 12), (12, 14), (13, 10)] {
            assert_eq!(
                text_in(&PYTHON, source, line, column).as_deref(),
                Some("Return the next value.\n\nIndented details.")
            );
        }
        assert_eq!(range_in(&PYTHON, source, 12, 14), range((9, 8), (13, 11)));
        assert_eq!(
            text_in(&PYTHON, source, 18, 6).as_deref(),
            Some("Single quoted.")
        );
        assert_eq!(
            text_in(&PYTHON, source, 22, 6).as_deref(),
            Some("First second.")
        );
        assert_eq!(range_in(&PYTHON, source, 22, 6), range((22, 4), (22, 23)));
    }

    #[test]
    fn python_strings_that_are_not_docstrings_have_no_hover() {
        let source = r#"value = 1
"""Not first in module."""


def assigned():
    text = """Assigned string."""


def later():
    pass
    """Second statement."""


def formatted():
    f"""Formatted {value}."""


def raw_bytes():
    b"""Bytes."""


if value:
    """Inside if."""


call("""Argument.""")
"#;
        for (line, column) in [(1, 5), (5, 15), (10, 8), (14, 8), (18, 8), (22, 8), (25, 9)] {
            assert!(
                text_in(&PYTHON, source, line, column).is_none(),
                "{line}:{column}"
            );
        }
    }

    #[test]
    fn python_docstrings_are_ignored_for_other_languages() {
        assert!(text_in(&BASH, "\"\"\"Not a docstring.\"\"\"\n", 0, 5).is_none());
    }

    #[test]
    fn bash_comments_are_grouped_and_trailing_comments_are_separate() {
        let source = "# Prepare the\n# build.\nmake # Trailing.\n";
        for line in 0..2 {
            assert_eq!(
                text_in(&BASH, source, line, 3).as_deref(),
                Some("Prepare the build.")
            );
            assert_eq!(range_in(&BASH, source, line, 3), range((0, 0), (1, 8)));
        }
        assert_eq!(text_in(&BASH, source, 2, 7).as_deref(), Some("Trailing."));
    }

    #[test]
    fn bash_expansions_strings_and_heredocs_are_not_comments() {
        let source = "# Header.\necho \"$#\" ${#name} '# quoted' \"a # b\"\ncat <<EOF\n# heredoc text\nEOF\nvalue=a#b\n";
        for (line, column) in [(1, 7), (1, 12), (1, 20), (1, 33), (3, 0), (3, 3), (5, 7)] {
            assert!(
                text_in(&BASH, source, line, column).is_none(),
                "{line}:{column}"
            );
        }
        assert_eq!(text_in(&BASH, source, 0, 3).as_deref(), Some("Header."));
    }
}
