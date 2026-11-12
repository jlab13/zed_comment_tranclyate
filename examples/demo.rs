//! Demo module for comment translation.
//!
//! Place the cursor on any comment in this file and run editor::Hover.

/// Return the next counter value.
///
/// # Examples
///
/// ```
/// assert_eq!(next_value(1), 2);
/// ```
pub fn next_value(current: u32) -> u32 {
    // Overflow wraps around instead of panicking.
    current.wrapping_add(1)
}

/* A block comment /* with a nested comment */ inside. */
pub const MARKER: &str = r#"// This is a raw string, not a comment"#;
