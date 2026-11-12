// Place the cursor on a comment and run editor::Hover.
// Adjacent line comments are translated together as one block.

/// Counter that only moves forward.
struct Counter {
    /// The current counter value.
    private(set) var value = 0

    /**
     Return the next value without changing the counter.
     */
    func next() -> Int {
        value + 1 // Overflow traps at runtime.
    }
}

/* A block comment /* with a nested comment */ inside. */
let marker = "// This is a string, not a comment: \(Counter().next())"
