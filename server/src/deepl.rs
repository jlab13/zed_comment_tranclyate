// Переклад через офіційний API DeepL (DeepL SE). Потрібен власний ключ; план Free має
// місячну квоту символів. Ключ передається лише в заголовку, ніколи в адресі.

use crate::i18n::tr;
use reqwest::{header, Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::OnceLock, time::Duration};

const FREE_BASE: &str = "https://api-free.deepl.com";
const PRO_BASE: &str = "https://api.deepl.com";
const USER_AGENT: &str = concat!("comment-translate-lsp/", env!("CARGO_PKG_VERSION"));
/// Змінна оточення з ключем, як в офіційних бібліотеках DeepL.
pub const KEY_VARIABLE: &str = "DEEPL_AUTH_KEY";
static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

fn key_rejected() -> &'static str {
    tr!(
        "DeepL rejected the key (HTTP 403). Check deepl_key_file or DEEPL_AUTH_KEY.",
        "DeepL відхилив ключ (HTTP 403). Перевірте deepl_key_file або DEEPL_AUTH_KEY."
    )
}

fn rate_limited() -> &'static str {
    tr!(
        "DeepL is rate-limiting requests (HTTP 429).",
        "DeepL обмежив кількість запитів (HTTP 429)."
    )
}

fn quota() -> &'static str {
    tr!(
        "The DeepL character quota for this month is used up (HTTP 456).",
        "Ліміт символів DeepL на цей місяць вичерпано (HTTP 456)."
    )
}

fn timed_out() -> &'static str {
    tr!("DeepL timed out.", "Час очікування DeepL минув.")
}

fn invalid() -> String {
    tr!(
        "Invalid response from DeepL.",
        "Некоректна відповідь DeepL."
    )
    .to_owned()
}

fn client() -> Result<&'static Client, String> {
    CLIENT
        .get_or_init(|| {
            Client::builder().referer(false).build().map_err(|_| {
                tr!(
                    "Could not create the DeepL HTTP client",
                    "Не вдалося створити HTTP-клієнт DeepL"
                )
                .into()
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// DeepL обмежив запити, вичерпав квоту або відхилив ключ: варто якийсь час його пропускати.
pub fn throttled(error: &str) -> bool {
    error == rate_limited() || error == quota() || error == key_rejected()
}

/// Читає ключ із файлу, а без файлу — зі змінної `DEEPL_AUTH_KEY`. `Ok(None)` — ключа немає.
pub fn load_key(
    file: Option<&str>,
    variable: impl Fn(&str) -> Option<String>,
    home: Option<PathBuf>,
) -> Result<Option<String>, String> {
    if let Some(file) = file {
        let path = match (file.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(file),
        };
        // Шлях не показуємо повністю: досить імені файлу, щоб зрозуміти помилку.
        let name = path.file_name().map_or_else(
            || file.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let key = std::fs::read_to_string(&path).map_err(|_| {
            tr!(
                format!("Could not read deepl_key_file ({name})."),
                format!("Не вдалося прочитати deepl_key_file ({name}).")
            )
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(tr!(
                format!("deepl_key_file ({name}) is empty."),
                format!("deepl_key_file ({name}) порожній.")
            ));
        }
        return Ok(Some(key.to_owned()));
    }
    Ok(variable(KEY_VARIABLE)
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty()))
}

/// Опис плану для журналу: ключі плану Free закінчуються на `:fx`.
pub fn plan(key: &str) -> &'static str {
    if key.ends_with(":fx") {
        "Free"
    } else {
        "Pro"
    }
}

fn base(key: &str) -> &'static str {
    if key.ends_with(":fx") {
        FREE_BASE
    } else {
        PRO_BASE
    }
}

fn authorized(builder: RequestBuilder, key: &str, deadline: Duration) -> RequestBuilder {
    builder
        .header(header::AUTHORIZATION, format!("DeepL-Auth-Key {key}"))
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, USER_AGENT)
        .timeout(deadline)
}

/// Код вихідної мови DeepL без варіанта; `None` — DeepL визначає мову сам.
fn source_code(tag: &str) -> Option<String> {
    let language = tag.split(['-', '_']).next().unwrap_or(tag);
    if language.eq_ignore_ascii_case("auto") {
        return None;
    }
    Some(match language.to_ascii_lowercase().as_str() {
        "no" | "nb" | "nn" => "NB".into(),
        other => other.to_ascii_uppercase(),
    })
}

/// Код цільової мови DeepL. Англійська й португальська потребують варіанта: його задає
/// регіон у `target_language` (`en-GB`, `pt-PT`), інакше американська та бразильська.
fn target_code(tag: &str) -> String {
    let mut parts = tag.split(['-', '_']);
    let language = parts.next().unwrap_or(tag).to_ascii_lowercase();
    let rest: Vec<_> = parts.map(str::to_ascii_uppercase).collect();
    let has = |value: &str| rest.iter().any(|part| part == value);
    match language.as_str() {
        "en" if ["GB", "IE", "AU", "NZ", "IN", "ZA"].iter().any(|r| has(r)) => "EN-GB".into(),
        "en" => "EN-US".into(),
        "pt" if has("PT") => "PT-PT".into(),
        "pt" => "PT-BR".into(),
        "zh" if has("HANT") || has("TW") || has("HK") || has("MO") => "ZH-HANT".into(),
        "zh" => "ZH-HANS".into(),
        "no" | "nb" | "nn" => "NB".into(),
        other => other.to_ascii_uppercase(),
    }
}

#[derive(Serialize)]
struct TranslateBody<'a> {
    text: [&'a str; 1],
    target_lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_lang: Option<String>,
}

fn translate_request(
    client: &Client,
    text: &str,
    source: &str,
    target: &str,
    key: &str,
    deadline: Duration,
) -> RequestBuilder {
    let body = TranslateBody {
        text: [text],
        target_lang: target_code(target),
        source_lang: source_code(source),
    };
    authorized(
        client.post(format!("{}/v2/translate", base(key))),
        key,
        deadline,
    )
    .json(&body)
}

/// Перекладає текст одним запитом: DeepL приймає до 128 КіБ, а коментар обмежено
/// `max_comment_chars`.
pub async fn translate(
    text: &str,
    source: &str,
    target: &str,
    key: &str,
    deadline: Duration,
) -> Result<String, String> {
    let request = translate_request(client()?, text, source, target, key, deadline);
    let (status, body) = send(request, deadline).await?;
    decode_translation(&body, status)
}

/// Використання квоти: перевіряє ключ і показує, скільки символів лишилося.
pub struct Usage {
    pub used: u64,
    pub limit: u64,
}

pub async fn usage(key: &str, deadline: Duration) -> Result<Usage, String> {
    let request = authorized(
        client()?.get(format!("{}/v2/usage", base(key))),
        key,
        deadline,
    );
    let (status, body) = send(request, deadline).await?;
    decode_usage(&body, status)
}

async fn send(request: RequestBuilder, deadline: Duration) -> Result<(u16, Vec<u8>), String> {
    let operation = async {
        let response = request.send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        let body = response.bytes().await.map_err(network_error)?;
        Ok((status, body.to_vec()))
    };
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| timed_out().to_owned())?
}

fn status_error(status: u16) -> String {
    match status {
        400 => tr!(
            "DeepL does not support this language pair or rejected the request (HTTP 400).",
            "DeepL не підтримує цю мовну пару або відхилив запит (HTTP 400)."
        )
        .into(),
        401 | 403 => key_rejected().into(),
        429 => rate_limited().into(),
        456 => quota().into(),
        500.. => tr!(
            format!("DeepL is temporarily unavailable (HTTP {status})."),
            format!("DeepL тимчасово недоступний (HTTP {status}).")
        ),
        _ => tr!(
            format!("DeepL returned HTTP {status}."),
            format!("DeepL повернув HTTP {status}.")
        ),
    }
}

#[derive(Deserialize)]
struct TranslateReply {
    translations: Vec<Translation>,
}

#[derive(Deserialize)]
struct Translation {
    text: String,
}

fn decode_translation(body: &[u8], status: u16) -> Result<String, String> {
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    let reply: TranslateReply = serde_json::from_slice(body).map_err(|_| invalid())?;
    let text: String = reply
        .translations
        .into_iter()
        .map(|translation| translation.text)
        .collect();
    if text.trim().is_empty() {
        return Err(tr!(
            "DeepL returned an empty translation.",
            "DeepL повернув порожній переклад."
        )
        .into());
    }
    Ok(text)
}

#[derive(Deserialize)]
struct UsageReply {
    character_count: u64,
    character_limit: u64,
}

fn decode_usage(body: &[u8], status: u16) -> Result<Usage, String> {
    if !(200..300).contains(&status) {
        return Err(status_error(status));
    }
    let reply: UsageReply = serde_json::from_slice(body).map_err(|_| invalid())?;
    Ok(Usage {
        used: reply.character_count,
        limit: reply.character_limit,
    })
}

fn network_error(error: reqwest::Error) -> String {
    // Не додаємо сирий текст помилки: він може містити адресу запиту.
    if error.is_timeout() {
        timed_out().into()
    } else {
        tr!(
            "Could not connect to DeepL. Check the network.",
            "Не вдалося з’єднатися з DeepL. Перевірте мережу."
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_keys_use_the_free_endpoint() {
        assert_eq!(base("abc:fx"), FREE_BASE);
        assert_eq!(base("abc"), PRO_BASE);
        assert_eq!(plan("abc:fx"), "Free");
        assert_eq!(plan("abc"), "Pro");
    }

    #[test]
    fn maps_language_codes_and_variants() {
        assert_eq!(source_code("auto"), None);
        assert_eq!(source_code("en"), Some("EN".into()));
        assert_eq!(source_code("zh-Hant"), Some("ZH".into()));
        assert_eq!(source_code("nb"), Some("NB".into()));
        for (target, expected) in [
            ("uk", "UK"),
            ("de-AT", "DE"),
            ("en", "EN-US"),
            ("en-GB", "EN-GB"),
            ("pt", "PT-BR"),
            ("pt-PT", "PT-PT"),
            ("zh-Hans", "ZH-HANS"),
            ("zh-TW", "ZH-HANT"),
            ("no", "NB"),
        ] {
            assert_eq!(target_code(target), expected, "{target}");
        }
    }

    #[test]
    fn request_keeps_the_key_in_the_header_and_text_in_the_body() {
        let client = Client::new();
        let request = translate_request(
            &client,
            "Call x0_ & check.",
            "auto",
            "uk",
            "secret:fx",
            Duration::from_secs(7),
        )
        .build()
        .unwrap();
        assert_eq!(request.method(), "POST");
        assert_eq!(
            request.url().as_str(),
            "https://api-free.deepl.com/v2/translate"
        );
        assert!(!request.url().as_str().contains("secret"));
        assert_eq!(
            request.headers()[header::AUTHORIZATION],
            "DeepL-Auth-Key secret:fx"
        );
        let body: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"text":["Call x0_ & check."],"target_lang":"UK"})
        );
        let request = translate_request(&client, "Hi", "en", "de", "k", Duration::from_secs(7))
            .build()
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["source_lang"], "EN");
    }

    #[test]
    fn decodes_translations_usage_and_errors() {
        let body = r#"{"translations":[{"detected_source_language":"EN","text":"Перший рядок.\nДругий."}]}"#;
        assert_eq!(
            decode_translation(body.as_bytes(), 200).unwrap(),
            "Перший рядок.\nДругий."
        );
        let usage =
            decode_usage(br#"{"character_count":1200,"character_limit":500000}"#, 200).unwrap();
        assert_eq!((usage.used, usage.limit), (1200, 500000));
        for (status, expected) in [
            (400, "400"),
            (403, "key"),
            (429, "429"),
            (456, "456"),
            (503, "503"),
        ] {
            assert!(
                decode_translation(&[], status)
                    .unwrap_err()
                    .contains(expected),
                "{status}"
            );
        }
        for body in [
            "",
            "{}",
            r#"{"translations":[]}"#,
            r#"{"translations":[{"text":" "}]}"#,
        ] {
            assert!(decode_translation(body.as_bytes(), 200).is_err(), "{body}");
        }
        assert!(throttled(&status_error(429)));
        assert!(throttled(&status_error(456)));
        assert!(throttled(&status_error(403)));
        assert!(!throttled(&status_error(400)));
    }

    #[test]
    fn loads_the_key_from_a_file_or_the_environment() {
        let directory =
            std::env::temp_dir().join(format!("comment-translate-deepl-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("deepl.key");
        std::fs::write(&file, "  file-key:fx\n").unwrap();
        let no_variable = |_: &str| None;
        assert_eq!(
            load_key(file.to_str(), no_variable, None).unwrap(),
            Some("file-key:fx".into())
        );
        // `~/` розгортається в домашній каталог.
        assert_eq!(
            load_key(Some("~/deepl.key"), no_variable, Some(directory.clone())).unwrap(),
            Some("file-key:fx".into())
        );
        let variable = |name: &str| (name == KEY_VARIABLE).then(|| " env-key ".to_owned());
        assert_eq!(
            load_key(None, variable, None).unwrap(),
            Some("env-key".into())
        );
        assert_eq!(load_key(None, no_variable, None).unwrap(), None);
        std::fs::write(&file, "\n").unwrap();
        assert!(load_key(file.to_str(), variable, None)
            .unwrap_err()
            .contains("empty"));
        let missing = load_key(Some("/nonexistent/secret-dir/deepl.key"), variable, None);
        let error = missing.unwrap_err();
        assert!(error.contains("deepl.key") && !error.contains("secret-dir"));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
