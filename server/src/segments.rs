// Захист коду від перекладу: блоки коду не надсилаються перекладачу,
// а код усередині речень замінюється мітками й повертається після перекладу.

use regex::{Captures, Regex};
use std::sync::LazyLock;

/// Блок коду, який залишається без перекладу.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeBlock {
    /// Огородження Markdown або Doxygen; закінчується рядком із цим маркером включно.
    Closed(&'static str),
    /// Приклад doctest Python; закінчується перед порожнім рядком.
    Doctest,
}

impl CodeBlock {
    /// Блок, що починається з рядка `line` (без початкових і кінцевих пробілів).
    pub fn start(line: &str) -> Option<Self> {
        if let Some(marker) = ["```", "~~~"].into_iter().find(|m| line.starts_with(m)) {
            return Some(Self::Closed(marker));
        }
        const DOXYGEN: [(&str, &str); 4] = [
            ("@code", "@endcode"),
            ("\\code", "\\endcode"),
            ("@verbatim", "@endverbatim"),
            ("\\verbatim", "\\endverbatim"),
        ];
        for (open, close) in DOXYGEN {
            // `@code{.c}` задає мову; `@codeline` чи `@code_value` — інші слова.
            if line
                .strip_prefix(open)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['{', ' ', '\t']))
            {
                return Some(Self::Closed(close));
            }
        }
        (line == ">>>" || line.starts_with(">>> ")).then_some(Self::Doctest)
    }

    /// Рядок уже не належить блоку: doctest закінчується перед порожнім рядком.
    pub fn ends_before(self, line: &str) -> bool {
        self == Self::Doctest && line.is_empty()
    }

    /// Рядок закриває блок і належить йому.
    pub fn ends_with(self, line: &str) -> bool {
        matches!(self, Self::Closed(marker) if line.starts_with(marker))
    }
}

/// Частина тексту коментаря.
#[derive(Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    /// Текст для перекладу разом із пробілами й переносами навколо нього.
    Prose(&'a str),
    /// Блок коду, який передається без змін.
    Code(&'a str),
}

/// Ділить текст на прозу й блоки коду. Частини разом дають точно вхідний текст.
pub fn split<'a>(text: &'a str) -> Vec<Segment<'a>> {
    let mut segments = Vec::new();
    let mut push = |code: bool, part: &'a str| {
        if !part.is_empty() {
            segments.push(if code {
                Segment::Code(part)
            } else {
                Segment::Prose(part)
            });
        }
    };
    let mut start = 0;
    let mut offset = 0;
    let mut block: Option<CodeBlock> = None;
    for line in text.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        let trimmed = line.trim();
        if let Some(current) = block {
            if current.ends_before(trimmed) {
                push(true, &text[start..line_start]);
                start = line_start;
                block = None;
            } else {
                if current.ends_with(trimmed) {
                    push(true, &text[start..offset]);
                    start = offset;
                    block = None;
                }
                continue;
            }
        }
        if let Some(opened) = CodeBlock::start(trimmed) {
            push(false, &text[start..line_start]);
            start = line_start;
            block = Some(opened);
        }
    }
    // Незакритий блок коду триває до кінця коментаря.
    push(block.is_some(), &text[start..]);
    segments
}

// Мітка схожа на ідентифікатор: перекладач бачить у реченні код і правильніше
// перекладає дієслова («Call x0_» → «Викличте», а з ⟦0⟧ — «Зателефонуйте»).
// Мітка виклику функції зберігає дужки: без них DeepL читає «Call x0_» як «зателефонуйте
// за номером», а «Call x0_()» — як виклик. Дужки, які перекладач відкинув, не заважають.
static LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[xX]([0-9]+)_(?:\(\))?").expect("Invalid label pattern"));

fn label(index: usize, code: &str) -> String {
    let call = code.trim_matches('`').ends_with(')');
    format!("x{index}_{}", if call { "()" } else { "" })
}

// Порядок гілок задає пріоритет, коли кілька правил починаються в одній позиції.
static CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
          (?P<ticks> `` (?: [^`] | `[^`] )+ `` | `[^`\n]+` )
        | (?P<url> https?://[^\s<>()\[\]{}`]+ )
        | (?P<command>
              [@\\] (?: t?param | retval | throws? | exception )
              (?: \[ [a-z,\ ]+ \] )? [\ \t]+ [A-Za-z_][A-Za-z0-9_]*
            | [@\\] [A-Za-z]+ )
        | (?P<field> : (?: param | type | raises? | returns? | rtype | var | ivar | cvar | keyword | kwarg )
              (?: [\ \t]+ [A-Za-z_][A-Za-z0-9_.]* )? : )
        | (?P<ident>
              [A-Za-z_][A-Za-z0-9_]* (?: (?: :: | \. | -> ) [A-Za-z_][A-Za-z0-9_]* )*
              (?: \( [^()\n]{0,60} \) )? )
        ",
    )
    .expect("Invalid code pattern")
});

/// Текст, у якому код замінено мітками `x0_`, `x1_`…
#[derive(Debug)]
pub struct Masked {
    pub text: String,
    spans: Vec<String>,
}

impl Masked {
    pub fn has_code(&self) -> bool {
        !self.spans.is_empty()
    }

    /// Поза мітками немає жодної літери, тому перекладати нічого.
    pub fn only_code(&self) -> bool {
        !LABEL
            .replace_all(&self.text, "")
            .chars()
            .any(char::is_alphabetic)
    }

    /// Повертає код на місце міток. `None`, якщо перекладач загубив, змінив або повторив мітку.
    pub fn restore(&self, translated: &str) -> Option<String> {
        let mut seen = vec![false; self.spans.len()];
        for captures in LABEL.captures_iter(translated) {
            let slot = seen.get_mut(captures[1].parse::<usize>().ok()?)?;
            if std::mem::replace(slot, true) {
                return None;
            }
        }
        if !seen.iter().all(|&found| found) {
            return None;
        }
        // Номери вже перевірено: кожен трапляється один раз і має свій фрагмент коду.
        let restored = LABEL.replace_all(translated, |captures: &Captures<'_>| {
            captures[1]
                .parse::<usize>()
                .ok()
                .and_then(|index| self.spans.get(index))
                .cloned()
                .unwrap_or_default()
        });
        Some(restored.into_owned())
    }
}

/// Замінює код у реченнях мітками. `None`, якщо мітки не можна однозначно повернути,
/// наприклад коли схожий на мітку текст лишився поза кодом.
pub fn mask(text: &str) -> Option<Masked> {
    let mut masked = String::with_capacity(text.len());
    let mut spans = Vec::new();
    let mut last = 0;
    for captures in CODE.captures_iter(text) {
        let Some((start, end)) = code_span(text, &captures) else {
            continue;
        };
        masked.push_str(&text[last..start]);
        let code = &text[start..end];
        masked.push_str(&label(spans.len(), code));
        spans.push(code.to_owned());
        last = end;
    }
    masked.push_str(&text[last..]);
    let masked = Masked {
        text: masked,
        spans,
    };
    (masked.restore(&masked.text).as_deref() == Some(text)).then_some(masked)
}

// Межі коду в знайденому збігу або `None`, якщо збіг — звичайне слово.
fn code_span(text: &str, captures: &Captures<'_>) -> Option<(usize, usize)> {
    let whole = captures.get(0)?;
    let (start, end) = (whole.start(), whole.end());
    if captures.name("url").is_some() {
        // Кінцевий розділовий знак належить реченню, а не адресі.
        let url = whole
            .as_str()
            .trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
        return Some((start, start + url.len()));
    }
    if captures.name("command").is_some() {
        // `user@example.com` — адреса, а не команда Doxygen.
        let word_before = text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        return (!word_before).then_some((start, end));
    }
    if captures.name("ident").is_some() && !looks_like_code(whole.as_str()) {
        return None;
    }
    Some((start, end))
}

// Ідентифікатор від слова відрізняє форма: `next_value`, `MAX_SIZE`, `nextValue`,
// `HashMap`, `std::mem`, `node->next`, `free()`.
fn looks_like_code(token: &str) -> bool {
    let (name, call) = match token.find('(') {
        Some(index) => (&token[..index], Some(&token[index..])),
        None => (token, None),
    };
    // `item(s)` і `value(es)` — множина в англійському тексті.
    let call = call.filter(|args| !matches!(*args, "(s)" | "(es)"));
    let camel_case = name
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0].is_ascii_lowercase() && pair[1].is_ascii_uppercase());
    name.chars().any(|ch| ch.is_ascii_alphanumeric())
        && (name.contains('_')
            || name.contains("::")
            || name.contains("->")
            || camel_case
            || call.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(char, &str)> {
        split(text)
            .into_iter()
            .map(|segment| match segment {
                Segment::Prose(part) => ('P', part),
                Segment::Code(part) => ('C', part),
            })
            .collect()
    }

    fn joined(text: &str) -> String {
        split(text)
            .into_iter()
            .map(|segment| match segment {
                Segment::Prose(part) | Segment::Code(part) => part,
            })
            .collect()
    }

    fn masked(text: &str) -> (String, Vec<String>) {
        let masked = mask(text).unwrap();
        (masked.text.clone(), masked.spans)
    }

    #[test]
    fn markdown_fences_are_code_and_split_is_lossless() {
        let text = "Return the value.\n\n# Examples\n\n```\nlet x = next_value(1);\n\nassert_eq!(x, 2);\n```\nDone.";
        assert_eq!(
            kinds(text),
            [
                ('P', "Return the value.\n\n# Examples\n\n"),
                (
                    'C',
                    "```\nlet x = next_value(1);\n\nassert_eq!(x, 2);\n```\n"
                ),
                ('P', "Done."),
            ]
        );
        assert_eq!(joined(text), text);
        assert_eq!(
            kinds("~~~rust\nfn f() {}\n~~~"),
            [('C', "~~~rust\nfn f() {}\n~~~")]
        );
    }

    #[test]
    fn doxygen_blocks_are_code() {
        let text =
            "Example:\n@code{.c}\nint x = 1;\n@endcode\n\\verbatim\nraw\n\\endverbatim\nEnd.";
        assert_eq!(
            kinds(text),
            [
                ('P', "Example:\n"),
                ('C', "@code{.c}\nint x = 1;\n@endcode\n"),
                ('C', "\\verbatim\nraw\n\\endverbatim\n"),
                ('P', "End."),
            ]
        );
        assert_eq!(
            kinds("@codeline is a word."),
            [('P', "@codeline is a word.")]
        );
    }

    #[test]
    fn doctest_ends_before_blank_line() {
        let text = "Add numbers.\n>>> add(1, 2)\n3\n\nMore text.";
        assert_eq!(
            kinds(text),
            [
                ('P', "Add numbers.\n"),
                ('C', ">>> add(1, 2)\n3\n"),
                ('P', "\nMore text."),
            ]
        );
        assert_eq!(joined(text), text);
    }

    #[test]
    fn unclosed_block_lasts_until_the_end() {
        assert_eq!(
            kinds("Text.\n```\ncode\n\nmore"),
            [('P', "Text.\n"), ('C', "```\ncode\n\nmore")]
        );
        assert!(kinds("").is_empty());
    }

    #[test]
    fn masks_inline_code_and_identifiers() {
        let (text, spans) =
            masked("Call `next_value()` or next_value(x), then read MAX_SIZE and nextValue.");
        assert_eq!(text, "Call x0_() or x1_(), then read x2_ and x3_.");
        assert_eq!(
            spans,
            ["`next_value()`", "next_value(x)", "MAX_SIZE", "nextValue"]
        );
        let (text, spans) = masked("Use std::mem::swap, node->next, HashMap and free().");
        assert_eq!(text, "Use x0_, x1_, x2_ and x3_().");
        assert_eq!(spans, ["std::mem::swap", "node->next", "HashMap", "free()"]);
        let (text, _) = masked("Use ``a ` b`` here.");
        assert_eq!(text, "Use x0_ here.");
    }

    #[test]
    fn masks_urls_without_trailing_punctuation() {
        let (text, spans) = masked("See https://example.com/a_b?x=1. Or (http://x.org).");
        assert_eq!(text, "See x0_. Or (x1_).");
        assert_eq!(spans, ["https://example.com/a_b?x=1", "http://x.org"]);
    }

    #[test]
    fn masks_documentation_commands_and_parameter_names() {
        let (text, spans) = masked("@param[in,out] node The node.\n\\return Zero.\n@brief Init.");
        assert_eq!(text, "x0_ The node.\nx1_ Zero.\nx2_ Init.");
        assert_eq!(spans, ["@param[in,out] node", "\\return", "@brief"]);
        let (text, spans) = masked(":param count: How many.\n:returns: The total.");
        assert_eq!(text, "x0_ How many.\nx1_ The total.");
        assert_eq!(spans, [":param count:", ":returns:"]);
    }

    #[test]
    fn leaves_ordinary_words_untouched() {
        for text in [
            "Return the next counter value.",
            "Read the item(s), e.g. a file.",
            "Mail user@example.com today.",
            "Version 1.2.3 of the API is NASA-approved.",
            "Перекласти коментар українською.",
        ] {
            assert_eq!(masked(text).0, text, "{text}");
        }
    }

    #[test]
    fn label_like_text_is_masked_as_code_and_restored() {
        let masked = mask("Build x86_64 and keep x0_ or X1_ as is.").unwrap();
        assert_eq!(masked.text, "Build x0_ and keep x1_ or x2_ as is.");
        assert_eq!(
            masked.restore(&masked.text).as_deref(),
            Some("Build x86_64 and keep x0_ or X1_ as is.")
        );
    }

    #[test]
    fn parentheses_after_plain_code_are_not_swallowed() {
        // `foo`() дав би «x0_()», і дужки зникли б після повернення коду; маскування відмовляється.
        assert!(mask("Call `foo`() first.").is_none());
        assert_eq!(
            mask("Call `foo()` first.").unwrap().text,
            "Call x0_() first."
        );
    }

    #[test]
    fn restores_code_in_translated_text() {
        let masked = mask("Call next_value() and MAX_SIZE.").unwrap();
        assert_eq!(masked.text, "Call x0_() and x1_.");
        assert_eq!(
            masked.restore("Викличте x0_() і x1_.").as_deref(),
            Some("Викличте next_value() і MAX_SIZE.")
        );
        // Перекладач міг відкинути дужки мітки: код усе одно повертається цілим.
        assert_eq!(
            masked.restore("Викличте x0_ і x1_.").as_deref(),
            Some("Викличте next_value() і MAX_SIZE.")
        );
        // Порядок міток може змінитися, а мітка на початку речення — отримати велику літеру.
        assert_eq!(
            masked.restore("X1_ і x0_.").as_deref(),
            Some("MAX_SIZE і next_value().")
        );
    }

    #[test]
    fn rejects_lost_changed_or_repeated_labels() {
        let masked = mask("Call next_value() and MAX_SIZE.").unwrap();
        for translated in [
            "Викличте x0_.",
            "Викличте x0_ і x0_.",
            "Викличте x 0_ і x1_.",
            "Викличте x0_ і x1_ та x2_.",
            "Викличте х0_ і x1_.",
            "Викличте [0] і [1].",
        ] {
            assert!(masked.restore(translated).is_none(), "{translated}");
        }
    }

    #[test]
    fn detects_text_without_words_outside_code() {
        assert!(mask("next_value()").unwrap().only_code());
        assert!(mask("`a`, `b`.").unwrap().only_code());
        assert!(mask("----").unwrap().only_code());
        assert!(!mask("Call next_value().").unwrap().only_code());
        assert!(!mask("next_value() повертає").unwrap().only_code());
        assert!(!mask("Plain text").unwrap().has_code());
    }
}
