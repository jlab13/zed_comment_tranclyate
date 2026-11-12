// Place the cursor on this comment and run editor::Hover.
// Adjacent line comments are translated together as one block.
#include "demo.h"

/**
 * @brief Return the next counter value.
 *
 * The current value stays unchanged:
 * 1. Read the current value
 * 2. Add one to it
 *
 * @param current The current counter value.
 * @return The next counter value.
 */
int next_value(int current) {
    const char *marker = "// This is a string, not a comment";
    (void)marker;
    return current + 1; // Overflow is not checked here.
}
