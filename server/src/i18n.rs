// Мова інтерфейсу сервера: англійська за замовчуванням, українська, коли переклад іде
// українською. Один процес LSP обслуговує одні налаштування, тож вистачає одного перемикача.

use std::sync::atomic::{AtomicBool, Ordering};

static UKRAINIAN: AtomicBool = AtomicBool::new(false);

/// Чи показувати інтерфейс українською для цільової мови `target`.
pub fn is_ukrainian(target: &str) -> bool {
    target
        .split(['-', '_'])
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("uk"))
}

/// Вибирає мову повідомлень за цільовою мовою перекладу.
pub fn set_for_target(target: &str) {
    UKRAINIAN.store(is_ukrainian(target), Ordering::Relaxed);
}

pub fn ukrainian() -> bool {
    UKRAINIAN.load(Ordering::Relaxed)
}

/// Текст інтерфейсу англійською або українською; обчислюється лише вибраний варіант.
macro_rules! tr {
    ($en:expr, $uk:expr $(,)?) => {
        if $crate::i18n::ukrainian() {
            $uk
        } else {
            $en
        }
    };
}
pub(crate) use tr;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ukrainian_interface_only_for_ukrainian_target() {
        for target in ["uk", "UK", "uk-UA", "uk_UA"] {
            assert!(is_ukrainian(target), "{target}");
        }
        for target in ["en", "de", "ru", "pl", "", "auto"] {
            assert!(!is_ukrainian(target), "{target}");
        }
    }

    #[test]
    fn interface_is_english_by_default() {
        // Тести не змінюють перемикач, тож він лишається в типовому стані.
        assert!(!ukrainian());
        assert_eq!(tr!("Translation", "Переклад"), "Translation");
    }
}
