use crate::deepl;
use crate::google;
use crate::i18n::tr;
use crate::locale::{self, TargetLanguage};
use crate::mymemory;
use crate::segments::{self, Segment};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, PoisonError},
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, time::timeout};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    MyMemory,
    DeepL,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Google => "Google Translate",
            Self::MyMemory => "MyMemory",
            Self::DeepL => "DeepL",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::MyMemory => "mymemory",
            Self::DeepL => "deepl",
        }
    }

    /// Запасні рушії без поля `fallback`, у тому самому порядку, що й у плагіні перекладу Omarchy.
    fn default_fallback(self) -> Vec<Provider> {
        match self {
            Self::DeepL => vec![Self::Google, Self::MyMemory],
            Self::Google => vec![Self::MyMemory],
            Self::MyMemory => Vec::new(),
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Сервіс перекладу. Без нього коментарі нікуди не надсилаються, а hover пояснює,
    /// як вибрати сервіс: текст коментаря отримує лише той сервіс, який вибрав користувач.
    pub provider: Option<Provider>,
    pub source_language: String,
    /// Мова перекладу; відсутнє поле або `auto` означає мову системної локалі.
    pub target_language: Option<String>,
    /// Запасні рушії по черзі: `"none"`, назва провайдера або список. Без поля
    /// DeepL переходить на Google і MyMemory, Google — на MyMemory.
    #[serde(deserialize_with = "deserialize_fallback")]
    pub fallback: Option<Vec<Provider>>,
    /// Адреса для MyMemory: піднімає анонімний денний ліміт.
    pub mymemory_email: Option<String>,
    /// Файл із ключем DeepL; без нього ключ береться зі змінної `DEEPL_AUTH_KEY`.
    pub deepl_key_file: Option<String>,
    pub timeout_seconds: u64,
    pub max_comment_chars: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: None,
            source_language: "auto".into(),
            target_language: None,
            fallback: None,
            mymemory_email: None,
            deepl_key_file: None,
            timeout_seconds: 60,
            max_comment_chars: 4000,
        }
    }
}

fn fallback_error() -> &'static str {
    tr!(
        "fallback must be \"none\", a provider name (google, mymemory, deepl), or a list of them",
        "fallback має бути \"none\", назвою провайдера (google, mymemory, deepl) або їх списком"
    )
}

fn deserialize_fallback<'de, D>(deserializer: D) -> Result<Option<Vec<Provider>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Setting {
        One(String),
        Many(Vec<Provider>),
    }
    let invalid = || serde::de::Error::custom(fallback_error());
    match Setting::deserialize(deserializer).map_err(|_| invalid())? {
        Setting::Many(list) => Ok(Some(list)),
        Setting::One(name) if name == "none" => Ok(Some(Vec::new())),
        Setting::One(name) => serde_json::from_value(serde_json::Value::String(name))
            .map(|provider| Some(vec![provider]))
            .map_err(|_| invalid()),
    }
}

fn missing_deepl_key() -> String {
    tr!(
        "No DeepL key: set deepl_key_file or the DEEPL_AUTH_KEY environment variable.",
        "Ключ DeepL не задано: вкажіть deepl_key_file або змінну оточення DEEPL_AUTH_KEY."
    )
    .into()
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.source_language.trim().is_empty() {
            return Err(tr!(
                "source_language must be auto or a language code such as en",
                "source_language має бути auto або кодом мови, наприклад en"
            )
            .into());
        }
        explicit_target(self.target_language.as_deref())?;
        if let (Some(provider), Some(fallback)) = (self.provider, &self.fallback) {
            for (index, other) in fallback.iter().enumerate() {
                if *other == provider || fallback[..index].contains(other) {
                    let id = provider.id();
                    return Err(tr!(
                        format!("fallback cannot contain the provider itself ({id}) or repeats"),
                        format!("fallback не може містити сам провайдер ({id}) чи повтори")
                    ));
                }
            }
        }
        validate_email(self.mymemory_email.as_deref())?;
        if !(1..=300).contains(&self.timeout_seconds)
            || !(1..=20000).contains(&self.max_comment_chars)
        {
            return Err("timeout_seconds: 1–300; max_comment_chars: 1–20000".into());
        }
        Ok(())
    }

    /// Чи вибрав користувач сервіс перекладу.
    pub fn configured(&self) -> bool {
        self.provider.is_some()
    }

    /// Провайдер і запасні рушії в порядку спроб; порожньо, доки провайдера не вибрано.
    pub fn chain(&self) -> Vec<Provider> {
        let Some(provider) = self.provider else {
            return Vec::new();
        };
        let fallback = self
            .fallback
            .clone()
            .unwrap_or_else(|| provider.default_fallback());
        std::iter::once(provider).chain(fallback).collect()
    }

    /// Ключ DeepL, якщо DeepL є серед рушіїв; `Err` пояснює, чому ключа немає.
    pub fn deepl_key(&self) -> Option<Result<String, String>> {
        if !self.chain().contains(&Provider::DeepL) {
            return None;
        }
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let key = deepl::load_key(
            self.deepl_key_file.as_deref(),
            |name| std::env::var(name).ok(),
            home,
        );
        Some(key.and_then(|key| key.ok_or_else(missing_deepl_key)))
    }

    /// Явна мова з налаштувань або мова системи, яку повертає `detect`.
    pub fn target_language(&self, detect: impl FnOnce() -> TargetLanguage) -> TargetLanguage {
        match explicit_target(self.target_language.as_deref()) {
            Ok(Some(code)) => TargetLanguage {
                code,
                origin: tr!("target_language setting", "налаштування target_language").into(),
            },
            _ => detect(),
        }
    }
}

fn validate_email(email: Option<&str>) -> Result<(), String> {
    match email {
        Some(email)
            if email.len() > 254
                || email.chars().any(char::is_whitespace)
                || !email
                    .split_once('@')
                    .is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.')) =>
        {
            Err(tr!(
                "mymemory_email must be an email address such as name@example.com",
                "mymemory_email має бути адресою пошти, наприклад name@example.com"
            )
            .into())
        }
        _ => Ok(()),
    }
}

fn target_error() -> String {
    tr!(
        "target_language must be a language code such as uk, en, or zh-Hans; auto or no field means the system language",
        "target_language має бути кодом мови, наприклад uk, en або zh-Hans; auto або відсутнє поле — мова системи"
    )
    .into()
}

/// Цільова мова із сирих налаштувань, ще до їх повної перевірки: за нею вибирається мова
/// повідомлень, щоб і помилки налаштувань були зрозумілі користувачу.
pub fn requested_target(options: Option<&serde_json::Value>) -> String {
    options
        .and_then(|options| options.get("target_language"))
        .and_then(serde_json::Value::as_str)
        .and_then(|target| explicit_target(Some(target)).ok().flatten())
        .unwrap_or_else(|| locale::system_language().code)
}

/// Явна цільова мова; `None` означає, що її треба взяти із системної локалі.
fn explicit_target(value: Option<&str>) -> Result<Option<String>, String> {
    match value.map(str::trim) {
        None => Ok(None),
        Some(tag) if tag.eq_ignore_ascii_case("auto") => Ok(None),
        Some(tag) => locale::normalize_tag(tag)
            .map(Some)
            .ok_or_else(target_error),
    }
}

#[tower_lsp::async_trait]
trait Engine: Send + Sync {
    async fn translate(&self, text: &str, config: &Config, target: &str) -> Result<String, String>;
}

/// Скільки пропускати сервіс після ліміту, вичерпаної квоти чи відхиленого ключа.
const PAUSE: Duration = Duration::from_secs(300);

struct ProviderEngine {
    /// Ключ DeepL або причина, чому його немає.
    deepl_key: Result<String, String>,
    // До цього моменту сервіс пропускається: він уже відмовляє, тож не смикаємо його.
    paused_until: std::sync::Mutex<HashMap<Provider, Instant>>,
}

impl ProviderEngine {
    fn new(deepl_key: Option<Result<String, String>>) -> Self {
        Self {
            deepl_key: deepl_key.unwrap_or_else(|| Err(missing_deepl_key())),
            paused_until: std::sync::Mutex::default(),
        }
    }

    fn paused(&self, provider: Provider) -> bool {
        self.paused_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&provider)
            .is_some_and(|until| Instant::now() < *until)
    }

    fn pause(&self, provider: Provider) {
        self.paused_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(provider, Instant::now() + PAUSE);
    }

    async fn call(
        &self,
        provider: Provider,
        text: &str,
        config: &Config,
        target: &str,
        deadline: Duration,
    ) -> Result<String, String> {
        let source = &config.source_language;
        match provider {
            Provider::Google => google::translate(text, source, target, deadline)
                .await
                .map(|reply| reply.translation),
            Provider::MyMemory => {
                let email = config.mymemory_email.as_deref();
                mymemory::translate(text, source, target, email, deadline).await
            }
            Provider::DeepL => match &self.deepl_key {
                Ok(key) => deepl::translate(text, source, target, key, deadline).await,
                Err(reason) => Err(reason.clone()),
            },
        }
    }
}

/// Сервіс обмежив запити, вичерпав квоту або відхилив ключ.
fn throttled(provider: Provider, error: &str) -> bool {
    match provider {
        Provider::Google => google::throttled(error),
        Provider::MyMemory => mymemory::throttled(error),
        Provider::DeepL => deepl::throttled(error),
    }
}

#[tower_lsp::async_trait]
impl Engine for ProviderEngine {
    async fn translate(&self, text: &str, config: &Config, target: &str) -> Result<String, String> {
        if same_language(&config.source_language, target) {
            return Ok(text.to_owned());
        }
        let chain = config.chain();
        if chain.is_empty() {
            // Hover сюди не доходить, але без провайдера текст точно нікуди не надсилається.
            return Err(tr!(
                "No translation provider is selected.",
                "Провайдера перекладу не вибрано."
            )
            .into());
        }
        // Кожна спроба отримує рівну частку часу, щоб повільний сервіс не з’їв увесь тайм-аут.
        let share = Duration::from_secs(config.timeout_seconds) / chain.len() as u32;
        let mut errors = Vec::new();
        for (index, &provider) in chain.iter().enumerate() {
            let next = chain.get(index + 1).copied();
            // Сервіс, що недавно відмовив, і DeepL без ключа пропускаються, поки є запасні.
            // Про відсутній ключ уже повідомлено в журналі під час запуску.
            let unavailable =
                self.paused(provider) || (provider == Provider::DeepL && self.deepl_key.is_err());
            if unavailable && next.is_some() {
                continue;
            }
            match self.call(provider, text, config, target, share).await {
                Ok(translation) => return Ok(translation),
                Err(error) => {
                    if throttled(provider, &error) {
                        self.pause(provider);
                    }
                    if let Some(next) = next {
                        // Stderr потрапляє в журнал мовного сервера Zed; stdout зайнятий протоколом LSP.
                        let next = next.name();
                        eprintln!(
                            "{}",
                            tr!(
                                format!("{error} Translating with {next}."),
                                format!("{error} Переклад через {next}.")
                            )
                        );
                    }
                    errors.push(error);
                }
            }
        }
        Err(errors.join("\n"))
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
        // Ключ DeepL читається один раз під час запуску; після зміни файлу перезапустіть сервер.
        let engine = Arc::new(ProviderEngine::new(config.deepl_key()));
        Self {
            config,
            target,
            engine,
            cache: Mutex::new(HashMap::new()),
            gate: Mutex::new(()),
        }
    }

    pub fn target(&self) -> &TargetLanguage {
        &self.target
    }

    /// Чи вибрав користувач сервіс перекладу; без нього hover лише пояснює, як його вибрати.
    pub fn configured(&self) -> bool {
        self.config.configured()
    }

    /// Перекладає прозу коментаря; блоки коду повертаються без змін і не надсилаються перекладачу.
    pub async fn translate(&self, text: &str) -> Result<String, String> {
        if text.chars().count() > self.config.max_comment_chars {
            return Err(tr!(
                "The comment exceeds max_comment_chars; no translation was requested",
                "Коментар перевищує max_comment_chars; переклад не запитувався"
            )
            .into());
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
            .map_err(|_| {
                let name = self.config.provider.map_or("LSP", Provider::name);
                tr!(
                    format!("{name} timed out"),
                    format!("Час очікування {name} минув")
                )
            })?
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

#[derive(Deserialize)]
struct DiagnosticRequest {
    provider: Option<Provider>,
    operation: Option<String>,
    text: Option<String>,
    source_language: Option<String>,
    target_language: Option<String>,
    mymemory_email: Option<String>,
    deepl_key_file: Option<String>,
    timeout_seconds: Option<u64>,
}

pub async fn diagnose(input: &str) -> Result<serde_json::Value, String> {
    let request: DiagnosticRequest = serde_json::from_str(input).map_err(|_| {
        tr!(
            "Invalid JSON request: check text, provider, and timeout_seconds",
            "Некоректний запит JSON: перевірте text, provider та timeout_seconds"
        )
    })?;
    // У CLI користувач сам вводить текст для перекладу, тому без поля provider береться Google.
    let provider = request.provider.unwrap_or(Provider::Google);
    let seconds = request.timeout_seconds.unwrap_or(60);
    if !(1..=300).contains(&seconds) {
        return Err(tr!(
            "timeout_seconds must be between 1 and 300",
            "timeout_seconds має бути в межах 1–300"
        )
        .into());
    }
    let target = explicit_target(request.target_language.as_deref())?
        .unwrap_or_else(|| locale::system_language().code);
    validate_email(request.mymemory_email.as_deref())?;
    let id = provider.id();
    let deadline = Duration::from_secs(seconds);
    let deepl_key = || -> Result<String, String> {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        deepl::load_key(
            request.deepl_key_file.as_deref(),
            |name| std::env::var(name).ok(),
            home,
        )?
        .ok_or_else(missing_deepl_key)
    };
    match request.operation.as_deref() {
        // Для DeepL перевірка справжня: запит використання квоти перевіряє і ключ.
        Some("check") if provider == Provider::DeepL => {
            let key = deepl_key()?;
            let usage = deepl::usage(&key, deadline).await?;
            return Ok(json!({
                "provider":id, "status":"ready", "plan":deepl::plan(&key),
                "character_count":usage.used, "character_limit":usage.limit,
                "target_language":target
            }));
        }
        Some("check") => {
            return Ok(
                json!({"provider":id, "status":"network_required", "target_language":target}),
            )
        }
        None | Some("translate") => {}
        _ => {
            return Err(tr!(
                "Unknown translation operation",
                "Невідома операція перекладу"
            )
            .into())
        }
    }
    let text = request.text.ok_or_else(|| {
        tr!(
            "Invalid JSON request: expected a text field",
            "Некоректний запит JSON: очікується поле text"
        )
        .to_owned()
    })?;
    if text.trim().is_empty() {
        return Err(tr!("No text to translate", "Немає тексту для перекладу").into());
    }
    let source = request.source_language.unwrap_or_else(|| "auto".into());
    if source.trim().is_empty() {
        return Err(tr!(
            "source_language cannot be empty",
            "source_language не може бути порожнім"
        )
        .into());
    }
    if same_language(&source, &target) {
        return Ok(
            json!({"provider":id, "translation":text, "source_language":source, "target_language":target}),
        );
    }
    // CLI перевіряє саме вибраний сервіс, тому запасні рушії тут не вмикаються.
    let (translation, source) = match provider {
        Provider::MyMemory => {
            let email = request.mymemory_email.as_deref();
            let translation = mymemory::translate(&text, &source, &target, email, deadline).await?;
            (translation, source)
        }
        Provider::DeepL => {
            let translation =
                deepl::translate(&text, &source, &target, &deepl_key()?, deadline).await?;
            (translation, source)
        }
        _ => {
            let reply = google::translate(&text, &source, &target, deadline).await?;
            (reply.translation, reply.source_language)
        }
    };
    Ok(
        json!({"provider":id, "translation":translation, "source_language":source, "target_language":target}),
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
    fn no_provider_is_selected_by_default() {
        let default: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(default.provider, None);
        assert!(!default.configured());
        assert!(default.chain().is_empty());
        let google: Config = serde_json::from_str(r#"{"provider":"google"}"#).unwrap();
        assert_eq!(google.provider, Some(Provider::Google));
        assert!(google.configured());
        assert_eq!(serde_json::to_value(Provider::Google).unwrap(), "google");
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
            .contains("timed out"));
        assert_eq!(engine.calls.load(Ordering::Relaxed), 1);
    }
    #[tokio::test]
    async fn google_diagnostics_return_same_language_text_without_network() {
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
        assert_eq!(*engine.requests.lock().unwrap(), ["Call x0_() and x1_."]);
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
            ["Call x0_().", "Call next_value()."]
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
    fn interface_language_comes_from_raw_settings_before_validation() {
        let options = json!({"target_language":"UK_ua", "fallback":"bing"});
        assert_eq!(requested_target(Some(&options)), "uk-UA");
        // Без явної мови береться системна; вона завжди непорожня.
        assert!(!requested_target(Some(&json!({"target_language":"???"}))).is_empty());
        assert!(!requested_target(None).is_empty());
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
        let text = ProviderEngine::new(None)
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

    #[test]
    fn default_chains_follow_the_omarchy_plugin_order() {
        use Provider::{DeepL, Google, MyMemory};
        let config = |json: &str| serde_json::from_str::<Config>(json).unwrap();
        assert!(config("{}").chain().is_empty());
        assert_eq!(
            config(r#"{"provider":"google"}"#).chain(),
            [Google, MyMemory]
        );
        assert_eq!(
            config(r#"{"provider":"deepl"}"#).chain(),
            [DeepL, Google, MyMemory]
        );
        assert_eq!(config(r#"{"provider":"mymemory"}"#).chain(), [MyMemory]);
        // Попередні значення `"mymemory"` і `"none"` залишаються дійсними.
        assert_eq!(
            config(r#"{"provider":"google","fallback":"none"}"#).chain(),
            [Google]
        );
        assert_eq!(
            config(r#"{"provider":"google","fallback":"mymemory"}"#).chain(),
            [Google, MyMemory]
        );
        // Запасні рушії без провайдера нічого не вмикають.
        assert!(config(r#"{"fallback":["google"]}"#).chain().is_empty());
        assert_eq!(
            config(r#"{"provider":"deepl","fallback":["mymemory"]}"#).chain(),
            [DeepL, MyMemory]
        );
        assert_eq!(
            config(r#"{"provider":"mymemory","fallback":"deepl"}"#).chain(),
            [MyMemory, DeepL]
        );
        for json in [
            r#"{"fallback":"bing"}"#,
            r#"{"fallback":["google", 1]}"#,
            r#"{"fallback":true}"#,
        ] {
            let error = serde_json::from_str::<Config>(json).err().unwrap();
            assert!(error.to_string().contains("fallback"), "{json}");
        }
    }

    #[test]
    fn rejects_fallback_to_itself_or_repeats() {
        for json in [
            r#"{"provider":"google","fallback":"google"}"#,
            r#"{"provider":"google","fallback":["mymemory","mymemory"]}"#,
            r#"{"provider":"deepl","fallback":["deepl"]}"#,
        ] {
            let config: Config = serde_json::from_str(json).unwrap();
            assert!(
                config.validate().unwrap_err().contains("fallback"),
                "{json}"
            );
        }
    }

    #[test]
    fn validates_mymemory_email() {
        for email in ["dev@example.com", "a.b+c@mail.example.org"] {
            let config = Config {
                mymemory_email: Some(email.into()),
                ..Config::default()
            };
            assert!(config.validate().is_ok(), "{email}");
        }
        for email in [
            "",
            "dev",
            "dev@localhost",
            "@example.com",
            "a b@example.com",
        ] {
            let config = Config {
                mymemory_email: Some(email.into()),
                ..Config::default()
            };
            assert!(
                config.validate().unwrap_err().contains("mymemory_email"),
                "{email}"
            );
        }
    }

    #[test]
    fn each_provider_is_paused_separately() {
        let engine = ProviderEngine::new(None);
        assert!(!engine.paused(Provider::Google));
        engine.pause(Provider::Google);
        assert!(engine.paused(Provider::Google));
        assert!(!engine.paused(Provider::MyMemory));
        engine
            .paused_until
            .lock()
            .unwrap()
            .insert(Provider::Google, Instant::now());
        assert!(!engine.paused(Provider::Google));
    }

    #[test]
    fn deepl_key_is_read_only_when_deepl_is_in_the_chain() {
        let config: Config =
            serde_json::from_str(r#"{"deepl_key_file":"/nonexistent/deepl.key"}"#).unwrap();
        assert!(config.deepl_key().is_none());
        let config: Config = serde_json::from_str(
            r#"{"provider":"deepl","deepl_key_file":"/nonexistent/deepl.key"}"#,
        )
        .unwrap();
        assert!(config
            .deepl_key()
            .unwrap()
            .unwrap_err()
            .contains("deepl_key_file"));
    }

    #[tokio::test]
    async fn deepl_without_a_key_fails_without_network() {
        let config = Config {
            provider: Some(Provider::DeepL),
            fallback: Some(Vec::new()),
            ..Config::default()
        };
        let error = ProviderEngine::new(Some(Err(missing_deepl_key())))
            .translate("Hello", &config, "uk")
            .await
            .unwrap_err();
        assert!(error.contains("DEEPL_AUTH_KEY"));
        assert!(diagnose(
            r#"{"provider":"deepl","operation":"check","deepl_key_file":"/nonexistent/deepl.key"}"#
        )
        .await
        .unwrap_err()
        .contains("deepl_key_file"));
    }

    #[tokio::test]
    async fn mymemory_diagnostics_work_without_network() {
        let value =
            diagnose(r#"{"provider":"mymemory","operation":"check","target_language":"uk"}"#)
                .await
                .unwrap();
        assert_eq!(value["provider"], "mymemory");
        assert_eq!(value["status"], "network_required");
        let value = diagnose(
            r#"{"provider":"mymemory","text":"Текст","source_language":"uk","target_language":"uk"}"#,
        )
        .await
        .unwrap();
        assert_eq!(value["translation"], "Текст");
        assert!(
            diagnose(r#"{"provider":"mymemory","text":"Hi","mymemory_email":"nope"}"#)
                .await
                .unwrap_err()
                .contains("mymemory_email")
        );
    }
}
