use crate::google;
#[cfg(all(target_os = "macos", feature = "apple"))]
use crate::native;
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
    pub timeout_seconds: u64,
    pub max_comment_chars: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: Provider::default(),
            source_language: "auto".into(),
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
        if !(1..=300).contains(&self.timeout_seconds)
            || !(1..=20000).contains(&self.max_comment_chars)
        {
            return Err("timeout_seconds: 1–300; max_comment_chars: 1–20000".into());
        }
        Ok(())
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
    async fn translate(&self, text: &str, config: &Config) -> Result<String, String>;
}

struct ProviderEngine;

#[tower_lsp::async_trait]
impl Engine for ProviderEngine {
    async fn translate(&self, text: &str, config: &Config) -> Result<String, String> {
        if source_is_ukrainian(&config.source_language) {
            return Ok(text.to_owned());
        }
        match config.provider {
            Provider::Google => google::translate(
                text,
                &config.source_language,
                Duration::from_secs(config.timeout_seconds),
            )
            .await
            .map(|reply| reply.translation),
            Provider::Apple => {
                #[cfg(all(target_os = "macos", feature = "apple"))]
                {
                    let payload = json!({"provider":"apple", "text":text, "source_language":config.source_language}).to_string();
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
    engine: Arc<dyn Engine>,
    cache: Mutex<HashMap<String, String>>,
    // Серіалізуємо переклади; повторний hover отримує результат із кешу.
    gate: Mutex<()>,
}

impl Translator {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            engine: Arc::new(ProviderEngine),
            cache: Mutex::new(HashMap::new()),
            gate: Mutex::new(()),
        }
    }

    pub async fn translate(&self, text: &str) -> Result<String, String> {
        if text.chars().count() > self.config.max_comment_chars {
            return Err("Коментар перевищує max_comment_chars; переклад не запитувався".into());
        }
        let operation = async {
            let _guard = self.gate.lock().await;
            if let Some(cached) = self.cache.lock().await.get(text).cloned() {
                return Ok(cached);
            }
            let translated = self.engine.translate(text, &self.config).await?;
            let mut cache = self.cache.lock().await;
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(text.to_owned(), translated.clone());
            Ok(translated)
        };
        timeout(Duration::from_secs(self.config.timeout_seconds), operation)
            .await
            .map_err(|_| format!("Час очікування {} минув", self.config.provider.name()))?
    }
}

fn source_is_ukrainian(source: &str) -> bool {
    source
        .split(['-', '_'])
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("uk"))
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
    if provider == Provider::Apple {
        #[cfg(all(target_os = "macos", feature = "apple"))]
        {
            let reply = native::request(input.to_owned(), Duration::from_secs(seconds)).await?;
            return serde_json::from_str(&reply)
                .map_err(|_| "Некоректний JSON від Apple Translation".into());
        }
        #[cfg(not(all(target_os = "macos", feature = "apple")))]
        {
            return Err(apple_unavailable());
        }
    }
    match request.operation.as_deref() {
        Some("check") => return Ok(json!({"provider":"google", "status":"network_required"})),
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
    if source_is_ukrainian(&source) {
        return Ok(json!({"provider":"google", "translation":text, "source_language":source}));
    }
    let reply = google::translate(&text, &source, Duration::from_secs(seconds)).await?;
    Ok(
        json!({"provider":"google", "translation":reply.translation, "source_language":reply.source_language}),
    )
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
        async fn translate(&self, _: &str, _: &Config) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(self.delay).await;
            if self.fail {
                Err("Моделі не готові".into())
            } else {
                Ok("Переклад".into())
            }
        }
    }

    fn mock(fail: bool, delay: Duration) -> (Translator, Arc<MockEngine>) {
        let engine = Arc::new(MockEngine {
            calls: AtomicUsize::new(0),
            fail,
            delay,
        });
        let mut translator = Translator::new(Config {
            timeout_seconds: 1,
            ..Config::default()
        });
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
        let value = diagnose(r#"{"provider":"google","text":"Текст 😀","source_language":"uk"}"#)
            .await
            .unwrap();
        assert_eq!(value["provider"], "google");
        assert_eq!(value["translation"], "Текст 😀");
    }
}
