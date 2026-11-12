// Визначення цільової мови перекладу за системною локаллю.

/// Мова, якщо системну локаль визначити не вдалося.
pub const FALLBACK_LANGUAGE: &str = "en";

/// Цільова мова перекладу разом із джерелом, звідки її взято.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetLanguage {
    /// Мовний тег BCP 47, наприклад `uk`, `de-AT` або `zh-Hans`.
    pub code: String,
    /// Пояснення для журналу LSP: налаштування, змінна оточення тощо.
    pub origin: String,
}

impl TargetLanguage {
    pub fn fallback() -> Self {
        Self {
            code: FALLBACK_LANGUAGE.into(),
            origin: "системну мову не визначено".into(),
        }
    }
}

/// Приводить мовний тег до вигляду BCP 47: `UK_ua` → `uk-UA`, `zh_hans` → `zh-Hans`.
/// Повертає `None` для значень, які не є мовним тегом, зокрема `C`, `POSIX` та `auto`.
pub fn normalize_tag(tag: &str) -> Option<String> {
    let mut parts = tag.trim().split(['-', '_']);
    let language = parts.next()?;
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut result = language.to_ascii_lowercase();
    for part in parts {
        if !(2..=8).contains(&part.len()) || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        result.push('-');
        if part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            // Письмо: Hans, Latn.
            result.push_str(&part[..1].to_ascii_uppercase());
            result.push_str(&part[1..].to_ascii_lowercase());
        } else if part.len() == 2 {
            // Регіон: UA, TW.
            result.push_str(&part.to_ascii_uppercase());
        } else {
            result.push_str(&part.to_ascii_lowercase());
        }
    }
    Some(result)
}

/// Мова з POSIX-локалі (`uk_UA.UTF-8@euro`) або тегу Apple (`uk-UA`).
/// Регіон відкидається, бо не змінює переклад; для китайської зберігається письмо.
pub fn language_from_locale(locale: &str) -> Option<String> {
    let tag = normalize_tag(locale.split(['.', '@']).next()?)?;
    let mut parts = tag.split('-');
    let language = parts.next()?;
    if language != "zh" {
        return Some(language.to_owned());
    }
    let traditional = parts.any(|part| matches!(part, "Hant" | "TW" | "HK" | "MO"));
    Some(if traditional { "zh-Hant" } else { "zh-Hans" }.into())
}

/// Код мови для Google і MyMemory: регіон відкидається, крім варіантів, які ці сервіси розрізняють.
pub fn web_language(tag: &str) -> String {
    let mut parts = tag.split(['-', '_']);
    let language = parts.next().unwrap_or(tag).to_ascii_lowercase();
    let rest: Vec<_> = parts.map(str::to_ascii_lowercase).collect();
    let has = |value: &str| rest.iter().any(|part| part == value);
    match language.as_str() {
        "zh" if has("hant") || has("tw") || has("hk") || has("mo") => "zh-TW".into(),
        "zh" => "zh-CN".into(),
        "pt" if has("pt") => "pt-PT".into(),
        "nb" | "nn" => "no".into(),
        _ => language,
    }
}

/// Визначає мову за змінними оточення POSIX; `get` дозволяє тестам не змінювати оточення процесу.
pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Option<TargetLanguage> {
    // LC_ALL переважає LC_MESSAGES, а LC_MESSAGES — LANG.
    let (name, locale) = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| {
            get(name)
                .filter(|value| !value.trim().is_empty())
                .map(|value| (name, value))
        })?;
    let language = language_from_locale(&locale)?;
    // GNU gettext бере пріоритетний список LANGUAGE, але ігнорує його для локалі C.
    if let Some(list) = get("LANGUAGE") {
        if let Some(code) = list.split(':').find_map(language_from_locale) {
            return Some(TargetLanguage {
                code,
                origin: format!("LANGUAGE={list}"),
            });
        }
    }
    Some(TargetLanguage {
        code: language,
        origin: format!("{name}={locale}"),
    })
}

/// Мова системи: на macOS — перша з AppleLanguages, далі змінні оточення, інакше англійська.
pub fn system_language() -> TargetLanguage {
    #[cfg(target_os = "macos")]
    if let Some(found) = apple_language() {
        return found;
    }
    from_env(|name| std::env::var(name).ok()).unwrap_or_else(TargetLanguage::fallback)
}

#[cfg(target_os = "macos")]
fn apple_language() -> Option<TargetLanguage> {
    // Zed, запущений не з термінала, часто не має LANG; мова інтерфейсу macOS — в AppleLanguages.
    // Утиліта defaults є в кожній macOS, тож Apple-фреймворки для цього не потрібні.
    let output = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let first = first_apple_language(&String::from_utf8_lossy(&output.stdout))?;
    Some(TargetLanguage {
        code: language_from_locale(&first)?,
        origin: format!("AppleLanguages: {first}"),
    })
}

/// Перший елемент масиву у форматі `defaults read`: `(\n    "uk-UA",\n    en\n)`.
#[cfg(any(target_os = "macos", test))]
fn first_apple_language(output: &str) -> Option<String> {
    output
        .trim()
        .strip_prefix('(')?
        .split([',', ')'])
        .map(|item| item.trim().trim_matches('"'))
        .find(|item| !item.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> Option<TargetLanguage> {
        let vars: HashMap<_, _> = pairs.iter().copied().collect();
        from_env(|name| vars.get(name).map(|value| value.to_string()))
    }

    #[test]
    fn normalizes_language_tags_to_bcp47_case() {
        assert_eq!(normalize_tag("uk").as_deref(), Some("uk"));
        assert_eq!(normalize_tag(" UK_ua ").as_deref(), Some("uk-UA"));
        assert_eq!(normalize_tag("zh_hans_cn").as_deref(), Some("zh-Hans-CN"));
        assert_eq!(normalize_tag("es-419").as_deref(), Some("es-419"));
        assert_eq!(normalize_tag("fil").as_deref(), Some("fil"));
    }

    #[test]
    fn rejects_values_that_are_not_language_tags() {
        for value in [
            "", "auto", "C", "POSIX", "u", "english", "uk UA", "uk--UA", "uk-", "укр",
        ] {
            assert_eq!(normalize_tag(value), None, "{value}");
        }
    }

    #[test]
    fn extracts_language_from_posix_and_apple_locales() {
        for (locale, expected) in [
            ("uk_UA.UTF-8", "uk"),
            ("de_DE@euro", "de"),
            ("pt_BR.utf8", "pt"),
            ("en-US", "en"),
            ("zh_CN.UTF-8", "zh-Hans"),
            ("zh_TW.UTF-8", "zh-Hant"),
            ("zh-Hant-HK", "zh-Hant"),
            ("zh", "zh-Hans"),
        ] {
            assert_eq!(language_from_locale(locale).as_deref(), Some(expected));
        }
        for locale in ["C", "C.UTF-8", "POSIX", ""] {
            assert_eq!(language_from_locale(locale), None, "{locale}");
        }
    }

    #[test]
    fn posix_locale_variables_follow_precedence() {
        let all = [
            ("LC_ALL", "de_DE.UTF-8"),
            ("LC_MESSAGES", "fr_FR.UTF-8"),
            ("LANG", "uk_UA.UTF-8"),
        ];
        assert_eq!(env(&all).unwrap().code, "de");
        assert_eq!(env(&all[1..]).unwrap().code, "fr");
        let lang = env(&all[2..]).unwrap();
        assert_eq!(lang.code, "uk");
        assert_eq!(lang.origin, "LANG=uk_UA.UTF-8");
        assert_eq!(
            env(&[("LC_ALL", " "), ("LANG", "pl_PL.UTF-8")])
                .unwrap()
                .code,
            "pl"
        );
    }

    #[test]
    fn language_list_has_priority_except_for_c_locale() {
        let preferred = env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "::uk:en")]).unwrap();
        assert_eq!(preferred.code, "uk");
        assert_eq!(preferred.origin, "LANGUAGE=::uk:en");
        assert_eq!(env(&[("LANG", "C.UTF-8"), ("LANGUAGE", "uk")]), None);
        assert_eq!(env(&[("LC_ALL", "C"), ("LANG", "uk_UA.UTF-8")]), None);
        assert_eq!(env(&[]), None);
    }

    #[test]
    fn parses_first_language_from_defaults_output() {
        let output = "(\n    \"uk-UA\",\n    \"en-US\"\n)\n";
        assert_eq!(first_apple_language(output).as_deref(), Some("uk-UA"));
        assert_eq!(
            first_apple_language("(\n    en,\n    de\n)").as_deref(),
            Some("en")
        );
        assert_eq!(first_apple_language("(\n)"), None);
        assert_eq!(first_apple_language("garbage"), None);
    }
}
