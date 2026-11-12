#!/usr/bin/env python3
"""Demo module for comment translation.

The shebang above is not translated.
"""

# Adjacent line comments are translated together
# as one block.
MARKER = "# This is a string, not a comment"


class Counter:
    """Counter that only moves forward."""

    def next_value(self, current: int) -> int:
        """Return the next counter value.

        The current value stays unchanged.
        """
        return current + 1  # Python integers never overflow.
