// Place the cursor on a comment and run editor::Hover.
// Adjacent line comments are translated together as one block.

/// Counter that only moves forward.
class Counter {
  /// The current counter value.
  int value = 0;

  /**
   * Return the next value without changing the counter.
   */
  int next() => value + 1; // Integers are 64-bit on native platforms.
}

/* A block comment /* with a nested comment */ inside. */
const marker = '// This is a string, not a comment: ${1 + 1}';
