use crate::google;
use crate::locale::{self, TargetLanguage};
#[cfg(all(target_os = "macos", feature = "apple"))]
use crate::native;
use crate::segments::{self, Segment};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::timeout};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Apple,
    #[default]
    Google,
}

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::Apple => "Apple Translation",
            Self::Google => "Google Translate",
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: Provider,
    pub source_language: String,
    /// Мова перекладу; відсутнє поле або `auto` означає мову системної локалі.
    pub target_language: Option<String>,
    pub timeout_seconds: u64,
    pub max_comment_chars: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: Provider::default(),
            source_language: "auto".into(),
            target_language: None,
            timeout_seconds: 60,
            max_comment_chars: 4000,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        #[cfg(not(all(target_os = "macos", feature = "apple")))]
        if self.provider == Provider::Apple {
            return Err(apple_unavailable());
        }
        if self.source_language.trim().is_empty() {
            return Err("source_language має бути auto або кодом мови, наприклад en".into());
        }
        explicit_target(self.target_language.as_deref())?;
        if !(1..=300).contains(&self.timeout_seconds)
            || !(1..=20000).contains(&self.max_comment_chars)
        {
            return Err("timeout_seconds: 1–300; max_comment_chars: 1–20000".into());
        }
        Ok(())
    }

    /// Явна мова з налаштувань або мова системи, яку повертає `detect`.
    pub fn target_language(&self, detect: impl FnOnce() -> TargetLanguage) -> TargetLanguage {
        match explicit_target(self.target_language.as_deref()) {
            Ok(Some(code)) => TargetLanguage {
                code,
                origin: "налаштування target_language".into(),
            },
            _ => detect(),
        }
    }
}

const TARGET_ERROR: &str =
    "target_language має бути кодом мови, наприклад uk, en або zh-Hans; auto або відсутнє поле — мова системи";

/// Явна цільова мова; `None` означає, що її треба взяти із системної локалі.
fn explicit_target(value: Option<&str>) -> Result<Option<String>, String> {
    match value.map(str::trim) {
        None => Ok(None),
        Some(tag) if tag.eq_ignore_ascii_case("auto") => Ok(None),
        Some(tag) => locale::normalize_tag(tag)
            .map(Some)
            .ok_or_else(|| TARGET_ERROR.into()),
    }
}

#[cfg(all(target_os = "macos", feature = "apple"))]
#[derive(Deserialize)]
struct Response {
    translation: Option<String>,
    error: Option<String>,
}

#[tower_lsp::async_trait]
trait Engine: Send + Sync {
    async fn translate(&self, text: &str, config: &Config, target: &str) -> Result<String, String>;
}

struct ProviderEngine;

#[tower_lsp::async_trait]
impl Engine for ProviderEngine {
    async fn translate(&self, text: &str, config: &Config, target: &str) -> Result<String, String> {
        if same_language(&config.source_language, target) {
            return Ok(text.to_owned());
        }
        match config.provider {
            Provider::Google => google::translate(
                text,
                &config.source_language,
                target,
                Duration::from_secs(config.timeout_seconds),
            )
            .await
            .map(|reply| reply.translation),
            Provider::Apple => {
                #[cfg(all(target_os = "macos", feature = "apple"))]
                {
                    let payload = json!({"provider":"apple", "text":text, "source_language":config.source_language, "target_language":target}).to_string();
                    let response =
                        native::request(payload, Duration::from_secs(config.timeout_seconds))
                            .await?;
                    let response: Response = serde_json::from_str(&response)
                        .map_err(|_| "Некоректна відповідь бібліотеки Apple Translation")?;
                    if let Some(error) = response.error {
                        return Err(error);
                    }
                    response
                        .translation
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| "Apple Translation повернув порожній переклад".into())
                }
                #[cfg(not(all(target_os = "macos", feature = "apple")))]
                {
                    Err(apple_unavailable())
                }
            }
        }
    }
}

pub struct Translator {
    config: Config,
    target: TargetLanguage,
    engine: Arc<dyn Engine>,
    cache: Mutex<HashMap<String, String>>,
    // Серіалізуємо переклади; повторний hover отримує результат із кешу.
    gate: Mutex<()>,
}

impl Translator {
    pub fn new(config: Config, target: TargetLanguage) -> Self {
        Self {
            config,
            target,
            engine: Arc::new(ProviderEngine),
            cache: Mutex::new(HashMap::new()),
            gate: Mutex::new(()),
        }
    }

    pub fn target(&self) -> &TargetLanguage {
        &self.target
    }

    /// Перекладає прозу коментаря; блоки коду повертаються без змін і не надсилаються перекладачу.
    pub async fn translate(&self, text: &str) -> Result<String, String> {
        if text.chars().count() > self.config.max_comment_chars {
            return Err("Коментар перевищує max_comment_chars; переклад не запитувався".into());
        }
        let operation = async {
            let _guard = self.gate.lock().await;
            let mut result = String::with_capacity(text.len());
            for segment in segments::split(text) {
                match segment {
                    Segment::Code(code) => result.push_str(code),
                    Segment::Prose(prose) => {
                        let core = prose.trim();
                        if core.is_empty() {
                            result.push_str(prose);
                            continue;
                        }
                        // Переноси навколо абзацу відокремлюють його від блоків коду.
                        result.push_str(&prose[..prose.len() - prose.trim_start().len()]);
                        result.push_str(&self.translate_prose(core).await?);
                        result.push_str(&prose[prose.trim_end().len()..]);
                    }
                }
            }
            Ok(result)
        };
        timeout(Duration::from_secs(self.config.timeout_seconds), operation)
            .await
            .map_err(|_| format!("Час очікування {} минув", self.config.provider.name()))?
    }

    // Кожен шматок прози кешується окремо: однаковий абзац не перекладається вдруге.
    async fn translate_prose(&self, text: &str) -> Result<String, String> {
        if let Some(cached) = self.cache.lock().await.get(text).cloned() {
            return Ok(cached);
        }
        let translated = match segments::mask(text) {
            Some(masked) if masked.only_code() => text.to_owned(),
            Some(masked) if masked.has_code() => {
                let reply = self.engine_translate(&masked.text).await?;
                match masked.restore(&reply) {
                    Some(restored) => restored,
                    // Перекладач зіпсував мітки: перекладаємо текст без приховування коду.
                    None => self.engine_translate(text).await?,
                }
            }
            _ => self.engine_translate(text).await?,
        };
        let mut cache = self.cache.lock().await;
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(text.to_owned(), translated.clone());
        Ok(translated)
    }

    async fn engine_translate(&self, text: &str) -> Result<String, String> {
        self.engine
            .translate(text, &self.config, &self.target.code)
            .await
    }
}

/// Мови збігаються без урахування регіону: `uk-UA` і `uk`; `auto` не збігається ні з чим.
fn same_language(source: &str, target: &str) -> bool {
    locale::language_from_locale(source)
        .is_some_and(|source| Some(source) == locale::language_from_locale(target))
}

#[cfg(not(all(target_os = "macos", feature = "apple")))]
fn apple_unavailable() -> String {
    "Apple Translation доступний лише на macOS зі збіркою apple. Встановіть provider: google."
        .into()
}

#[derive(Deserialize)]
struct DiagnosticRequest {
    provider: Option<Provider>,
    operation: Option<String>,
    text: Option<String>,
    source_language: Option<String>,
    target_language: Option<String>,
    timeout_seconds: Option<u64>,
}

pub async fn diagnose(input: &str) -> Result<serde_json::Value, String> {
    let request: DiagnosticRequest = serde_json::from_str(input)
        .map_err(|_| "Некоректний запит JSON: перевірте text, provider та timeout_seconds")?;
    let provider = request.provider.unwrap_or_default();
    let seconds = request.timeout_seconds.unwrap_or(60);
    if !(1..=300).contains(&seconds) {
        return Err("timeout_seconds має бути в межах 1–300".into());
    }
    let target = explicit_target(request.target_language.as_deref())?
        .unwrap_or_else(|| locale::system_language().code);
    if provider == Provider::Apple {
        #[cfg(all(target_os = "macos", feature = "apple"))]
        {
            // Swift отримує вже визначену мову, щоб CLI і LSP поводилися однаково.
            let mut payload: serde_json::Value = serde_json::from_str(input).map_err(|_| {
                "Некоректний запит JSON: перевірте text, provider та timeout_seconds"
            })?;
            with_target(&mut payload, &target);
            let reply = native::request(payload.to_string(), Duration::from_secs(seconds)).await?;
            let mut reply: serde_json::Value = serde_json::from_str(&reply)
                .map_err(|_| "Некоректний JSON від Apple Translation")?;
            with_target(&mut reply, &target);
            return Ok(reply);
        }
        #[cfg(not(all(target_os = "macos", feature = "apple")))]
        {
            return Err(apple_unavailable());
        }
    }
    match request.operation.as_deref() {
        Some("check") => {
            return Ok(
                json!({"provider":"google", "status":"network_required", "target_language":target}),
            )
        }
        None | Some("translate") => {}
        _ => return Err("Невідома операція перекладу".into()),
    }
    let text = request
        .text
        .ok_or("Некоректний запит JSON: очікується поле text")?;
    if text.trim().is_empty() {
        return Err("Немає тексту для перекладу".into());
    }
    let source = request.source_language.unwrap_or_else(|| "auto".into());
    if source.trim().is_empty() {
        return Err("source_language не може бути порожнім".into());
    }
    if same_language(&source, &target) {
        return Ok(
            json!({"provider":"google", "translation":text, "source_language":source, "target_language":target}),
        );
    }
    let reply = google::translate(&text, &source, &target, Duration::from_secs(seconds)).await?;
    Ok(
        json!({"provider":"google", "translation":reply.translation, "source_language":reply.source_language, "target_language":target}),
    )
}

#[cfg(all(target_os = "macos", feature = "apple"))]
fn with_target(value: &mut serde_json::Value, target: &str) {
    if let Some(object) = value.as_object_mut() {
        object.insert("target_language".into(), target.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockEngine {
        calls: AtomicUsize,
        fail: bool,
        delay: Duration,
    }

    #[tower_lsp::async_trait]
    impl Engine for MockEngine {
        async fn translate(&self, _: &str, _: &Config, _: &str) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(self.delay).await;
            if self.fail {
                Err("Моделі не готові".into())
            } else {
                Ok("Переклад".into())
            }
        }
    }

    // Записує кожен запит і відповідає функцією `reply`.
    struct RecordingEngine {
        requests: std::sync::Mutex<Vec<String>>,
        reply: fn(&str) -> String,
    }

    #[tower_lsp::async_trait]
    impl Engine for RecordingEngine {
        async fn translate(&self, text: &str, _: &Config, _: &str) -> Result<String, String> {
            self.requests.lock().unwrap().push(text.to_owned());
            Ok((self.reply)(text))
        }
    }

    fn recording(reply: fn(&str) -> String) -> (Translator, Arc<RecordingEngine>) {
        let engine = Arc::new(RecordingEngine {
            requests: std::sync::Mutex::new(Vec::new()),
            reply,
        });
        let mut translator = Translator::new(Config::default(), TargetLanguage::fallback());
        translator.engine = engine.clone();
        (translator, engine)
    }

    // Імітує переклад, що зберігає мітки: лише замінює англійські слова.
    fn fake_ukrainian(text: &str) -> String {
        text.replace("Call", "Викличте")
            .replace("Return the value", "Повернути значення")
            .replace("and", "і")
            .replace("Done", "Готово")
    }

    fn mock(fail: bool, delay: Duration) -> (Translator, Arc<MockEngine>) {
        let engine = Arc::new(MockEngine {
            calls: AtomicUsize::new(0),
            fail,
            delay,
        });
        let mut translator = Translator::new(
            Config {
                timeout_seconds: 1,
                ..Config::default()
            },
            TargetLanguage::fallback(),
        );
        translator.engine = engine.clone();
        (translator, engine)
    }

    #[test]
    fn validates_configuration() {
        assert!(Config::default().validate().is_ok());
        assert!(Config {
            source_language: String::new(),
            ..Config::default()
        }
        .validate()
        .is_err());
        assert!(Config {
            timeout_seconds: 0,
            ..Config::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn rejects_removed_provider_options() {
        for json in [
            r#"{"endpoint":"http://localhost"}"#,
            r#"{"model":"example"}"#,
            r#"{"helper_path":"/tmp/old-helper"}"#,
        ] {
            assert!(serde_json::from_str::<Config>(json).is_err());
        }
    }

    #[test]
    fn provider_defaults_to_google_on_all_platforms() {
        let default: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(default.provider, Provider::Google);
        let google: Config = serde_json::from_str(r#"{"provider":"google"}"#).unwrap();
        assert_eq!(google.provider, Provider::Google);
        assert_eq!(serde_json::to_value(google.provider).unwrap(), "google");
        assert!(serde_json::from_str::<Config>(r#"{"provider":"unknown"}"#).is_err());
    }

    #[tokio::test]
    async fn caches_successful_translations() {
        let (translator, engine) = mock(false, Duration::ZERO);
        assert_eq!(
            translator.translate("Return value").await.unwrap(),
            "Переклад"
        );
        assert_eq!(
            translator.translate("Return value").await.unwrap(),
            "Переклад"
        );
        assert_eq!(engine.calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn does_not_cache_errors() {
        let (translator, engine) = mock(true, Duration::ZERO);
        assert!(translator.translate("Return value").await.is_err());
        assert!(translator.translate("Return value").await.is_err());
        assert_eq!(engine.calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn limits_input_before_calling_native_code() {
        let (translator, engine) = mock(false, Duration::ZERO);
        assert!(translator
            .translate(&"x".repeat(4001))
            .await
            .unwrap_err()
            .contains("max_comment_chars"));
        assert_eq!(engine.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn times_out_without_terminating_the_server() {
        let (translator, engine) = mock(false, Duration::from_secs(2));
        assert!(translator
            .translate("Return value")
            .await
            .unwrap_err()
            .contains("Час очікування"));
        assert_eq!(engine.calls.load(Ordering::Relaxed), 1);
    }
    #[cfg(not(all(target_os = "macos", feature = "apple")))]
    #[tokio::test]
    async fn apple_selection_fails_clearly_without_native_support() {
        let config = Config {
            provider: Provider::Apple,
            ..Config::default()
        };
        assert!(config.validate().unwrap_err().contains("macOS"));
        assert!(diagnose(r#"{"provider":"apple","text":"Hello"}"#)
            .await
            .unwrap_err()
            .contains("provider: google"));
    }

    #[tokio::test]
    async fn google_diagnostics_do_not_need_apple_models() {
        let value = diagnose(
            r#"{"provider":"google","text":"Текст 😀","source_language":"uk","target_language":"uk"}"#,
        )
        .await
        .unwrap();
        assert_eq!(value["provider"], "google");
        assert_eq!(value["translation"], "Текст 😀");
        assert_eq!(value["target_language"], "uk");
    }

    #[tokio::test]
    async fn code_blocks_are_never_sent_and_are_kept_verbatim() {
        let (translator, engine) = recording(fake_ukrainian);
        let text = "Return the value.\n\n```\nlet value = next_value(1);\n```\nDone.";
        assert_eq!(
            translator.translate(text).await.unwrap(),
            "Повернути значення.\n\n```\nlet value = next_value(1);\n```\nГотово."
        );
        assert_eq!(
            *engine.requests.lock().unwrap(),
            ["Return the value.", "Done."]
        );
    }

    #[tokio::test]
    async fn inline_code_is_masked_and_restored() {
        let (translator, engine) = recording(fake_ukrainian);
        assert_eq!(
            translator
                .translate("Call next_value() and `MAX_SIZE`.")
                .await
                .unwrap(),
            "Викличте next_value() і `MAX_SIZE`."
        );
        assert_eq!(*engine.requests.lock().unwrap(), ["Call x0_ and x1_."]);
    }

    #[tokio::test]
    async fn broken_labels_fall_back_to_unmasked_text() {
        let (translator, engine) = recording(|text| text.replace("x0_", "x 0"));
        assert_eq!(
            translator.translate("Call next_value().").await.unwrap(),
            "Call next_value()."
        );
        assert_eq!(
            *engine.requests.lock().unwrap(),
            ["Call x0_.", "Call next_value()."]
        );
    }

    #[tokio::test]
    async fn code_only_prose_is_not_sent() {
        let (translator, engine) = recording(fake_ukrainian);
        let text = "next_value()\n\n```\ncode\n```";
        assert_eq!(translator.translate(text).await.unwrap(), text);
        assert!(engine.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn each_paragraph_is_cached_separately() {
        let (translator, engine) = recording(fake_ukrainian);
        translator
            .translate("Return the value.\n```\na\n```\nDone.")
            .await
            .unwrap();
        translator
            .translate("Done.\n```\nb\n```\nReturn the value.")
            .await
            .unwrap();
        assert_eq!(engine.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn validates_target_language() {
        for target in ["uk", "DE_at", "zh-Hant", "auto", " AUTO "] {
            let config = Config {
                target_language: Some(target.into()),
                ..Config::default()
            };
            assert!(config.validate().is_ok(), "{target}");
        }
        for target in ["", "C", "українська", "uk ua"] {
            let config = Config {
                target_language: Some(target.into()),
                ..Config::default()
            };
            assert!(config.validate().unwrap_err().contains("target_language"));
        }
    }

    #[test]
    fn explicit_target_language_overrides_system_locale() {
        let config: Config = serde_json::from_str(r#"{"target_language":"de_AT"}"#).unwrap();
        let target = config.target_language(|| panic!("локаль не потрібна"));
        assert_eq!(target.code, "de-AT");
        assert!(target.origin.contains("target_language"));
    }

    #[test]
    fn missing_or_auto_target_language_uses_system_locale() {
        let detected = || TargetLanguage {
            code: "pl".into(),
            origin: "LANG=pl_PL.UTF-8".into(),
        };
        for json in ["{}", r#"{"target_language":"auto"}"#] {
            let config: Config = serde_json::from_str(json).unwrap();
            assert_eq!(config.target_language(detected), detected());
        }
    }

    #[test]
    fn compares_languages_without_region() {
        assert!(same_language("uk", "uk-UA"));
        assert!(same_language("EN_us", "en"));
        assert!(!same_language("auto", "uk"));
        assert!(!same_language("en", "uk"));
        assert!(!same_language("zh-Hant", "zh-Hans"));
    }

    #[tokio::test]
    async fn same_source_and_target_skip_the_provider() {
        let config = Config {
            source_language: "de".into(),
            ..Config::default()
        };
        let text = ProviderEngine
            .translate("Der Wert 😀", &config, "de-AT")
            .await
            .unwrap();
        assert_eq!(text, "Der Wert 😀");
    }

    #[tokio::test]
    async fn diagnostics_report_target_language_and_reject_invalid_values() {
        let value = diagnose(r#"{"operation":"check","target_language":"PL"}"#)
            .await
            .unwrap();
        assert_eq!(value["status"], "network_required");
        assert_eq!(value["target_language"], "pl");
        assert!(
            diagnose(r#"{"text":"Hello","target_language":"Polish language"}"#)
                .await
                .unwrap_err()
                .contains("target_language")
        );
    }
}
