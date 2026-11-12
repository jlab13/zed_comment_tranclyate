// Place the cursor on a comment and run editor::Hover.
// Adjacent line comments are translated together as one block.
#include <string>

/// Counter that only moves forward.
class Counter {
public:
    /**
     * @brief Return the next value without changing the counter.
     * @return The next counter value.
     */
    int next() const { return value_ + 1; }

private:
    int value_ = 0; // The counter starts at zero.
};

// A raw string with a comment marker is not a comment.
const std::string kMarker = R"(// This is a raw string, not a comment)";
