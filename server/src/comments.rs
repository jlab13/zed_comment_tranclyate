use crate::text::{byte_offset, clean, paragraphs, position};
use tower_lsp::lsp_types::{Position, Range};

pub struct Comment {
    pub text: String,
    pub range: Range,
}

pub fn at(source: &str, cursor: Position) -> Option<Comment> {
    let offset = byte_offset(source, cursor.line, cursor.character)?;
    if offset >= source.len() {
        return None;
    }
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c::LANGUAGE.into()).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    let node = comment_node(root, offset)?;
    if offset >= comment_end(source, node) {
        return None;
    }
    let mut first = node;
    let mut last = node;
    let mut parts = std::collections::VecDeque::from([clean(&source[node.byte_range()])]);
    if standalone_line_comment(source, node) {
        // Об’єднуємо лише сусідні рядки коментарів без коду перед //.
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
            let Some(previous) = comment_on_line(source, root, previous_start) else {
                break;
            };
            if previous.end_byte() > first.start_byte() {
                break;
            }
            parts.push_front(clean(&source[previous.byte_range()]));
            first = previous;
        }
        while let Some((suffix, _)) = source[last.end_byte()..].split_once('\n') {
            if !suffix.trim().is_empty() {
                break;
            }
            let next_start = last.end_byte() + suffix.len() + 1;
            let Some(next) = comment_on_line(source, root, next_start) else {
                break;
            };
            parts.push_back(clean(&source[next.byte_range()]));
            last = next;
        }
    }
    let text = paragraphs(&parts.into_iter().collect::<Vec<_>>().join("\n"));
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

fn comment_end(source: &str, node: tree_sitter::Node<'_>) -> usize {
    // Tree-sitter включає CR у кінець //, але LSP-координата закінчується до CRLF.
    node.end_byte() - usize::from(source[..node.end_byte()].ends_with('\r'))
}

fn comment_node(root: tree_sitter::Node<'_>, offset: usize) -> Option<tree_sitter::Node<'_>> {
    let mut node = root.descendant_for_byte_range(offset, offset + 1)?;
    while node.kind() != "comment" {
        node = node.parent()?;
    }
    (offset < node.end_byte()).then_some(node)
}

fn standalone_line_comment(source: &str, node: tree_sitter::Node<'_>) -> bool {
    let line_start = source[..node.start_byte()]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    source[node.byte_range()].starts_with("//")
        && source[line_start..node.start_byte()].trim().is_empty()
}

fn comment_on_line<'tree>(
    source: &str,
    root: tree_sitter::Node<'tree>,
    start: usize,
) -> Option<tree_sitter::Node<'tree>> {
    let line = source.get(start..)?.split('\n').next()?;
    let trimmed = line.trim_start();
    if !trimmed.starts_with("//") {
        return None;
    }
    let offset = start + line.len() - trimmed.len();
    let node = comment_node(root, offset)?;
    // Перевіряємо дерево, щоб не захопити // всередині рядка або /* ... */.
    (node.start_byte() == offset && standalone_line_comment(source, node)).then_some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
