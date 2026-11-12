// Переклад через публічний API MyMemory (Translated S.r.l.): без ключа, з денним лімітом.

use crate::i18n::tr;
use crate::locale;
use reqwest::{header, Client, Url};
use serde::Deserialize;
use serde_json::Value;
use std::{sync::OnceLock, time::Duration};

const ENDPOINT: &str = "https://api.mymemory.translated.net/get";
// MyMemory відхиляє запити довші за 500 символів; рахуємо байти UTF-8 із запасом.
const MAX_QUERY_BYTES: usize = 450;
const USER_AGENT: &str = concat!("comment-translate-lsp/", env!("CARGO_PKG_VERSION"));
static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

fn client() -> Result<&'static Client, String> {
    CLIENT
        .get_or_init(|| {
            Client::builder().referer(false).build().map_err(|_| {
                tr!(
                    "Could not create the MyMemory HTTP client",
                    "Не вдалося створити HTTP-клієнт MyMemory"
                )
                .into()
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Перекладає текст шматками, кожен із яких уміщується в ліміт одного запиту.
pub async fn translate(
    text: &str,
    source: &str,
    target: &str,
    email: Option<&str>,
    deadline: Duration,
) -> Result<String, String> {
    let client = client()?;
    let operation = async {
        let mut result = String::with_capacity(text.len());
        for piece in chunks(text, MAX_QUERY_BYTES) {
            let core = piece.trim();
            if core.is_empty() {
                result.push_str(piece);
                continue;
            }
            // MyMemory обрізає пробіли й переноси на краях, тож повертаємо їх самі.
            result.push_str(&piece[..piece.len() - piece.trim_start().len()]);
            result.push_str(&fetch(client, url(core, source, target, email), deadline).await?);
            result.push_str(&piece[piece.trim_end().len()..]);
        }
        Ok(result)
    };
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| timed_out().to_owned())?
}

fn url(text: &str, source: &str, target: &str, email: Option<&str>) -> Url {
    let mut url = Url::parse(ENDPOINT).expect("Invalid built-in MyMemory URL");
    let source = if source.eq_ignore_ascii_case("auto") {
        "autodetect".to_owned()
    } else {
        locale::web_language(source)
    };
    let pair = format!("{source}|{}", locale::web_language(target));
    url.query_pairs_mut()
        .append_pair("q", text)
        .append_pair("langpair", &pair);
    if let Some(email) = email {
        // Адреса піднімає анонімний денний ліміт MyMemory.
        url.query_pairs_mut().append_pair("de", email);
    }
    url
}

async fn fetch(client: &Client, url: Url, deadline: Duration) -> Result<String, String> {
    let response = client
        .get(url)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, USER_AGENT)
        .timeout(deadline)
        .send()
        .await
        .map_err(network_error)?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.map_err(network_error)?;
    decode(&bytes, status)
}

#[derive(Deserialize)]
struct Reply {
    #[serde(rename = "responseData")]
    data: Option<Data>,
    #[serde(rename = "responseStatus")]
    status: Option<Value>,
    #[serde(rename = "responseDetails")]
    details: Option<Value>,
    #[serde(rename = "quotaFinished")]
    quota_finished: Option<bool>,
}

#[derive(Deserialize)]
struct Data {
    #[serde(rename = "translatedText")]
    text: Option<String>,
}

fn quota() -> &'static str {
    tr!(
        "The MyMemory daily quota is used up. Set mymemory_email to raise it.",
        "Денний ліміт MyMemory вичерпано. Задайте mymemory_email, щоб збільшити ліміт."
    )
}

fn rate_limited() -> &'static str {
    tr!(
        "MyMemory is rate-limiting requests (HTTP 429).",
        "MyMemory обмежив кількість запитів (HTTP 429)."
    )
}

fn timed_out() -> &'static str {
    tr!("MyMemory timed out.", "Час очікування MyMemory минув.")
}

/// MyMemory обмежив запити або вичерпав денний ліміт: варто якийсь час його пропускати.
pub fn throttled(error: &str) -> bool {
    error == rate_limited() || error == quota()
}

fn decode(body: &[u8], status: u16) -> Result<String, String> {
    if status == 429 {
        return Err(rate_limited().into());
    }
    let invalid = || {
        tr!(
            "Invalid response from MyMemory.",
            "Некоректна відповідь MyMemory."
        )
        .to_owned()
    };
    let reply: Reply = match serde_json::from_slice(body) {
        Ok(reply) => reply,
        Err(_) if !(200..300).contains(&status) => {
            return Err(tr!(
                format!("MyMemory returned HTTP {status}."),
                format!("MyMemory повернув HTTP {status}.")
            ))
        }
        Err(_) => return Err(invalid()),
    };
    if reply.quota_finished == Some(true) {
        return Err(quota().into());
    }
    // Статус приходить числом або рядком.
    let code = reply.status.as_ref().and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    });
    if code != Some(200) || !(200..300).contains(&status) {
        let details = reply
            .details
            .as_ref()
            .and_then(Value::as_str)
            .filter(|details| !details.trim().is_empty())
            .map(|details| details.chars().take(200).collect::<String>());
        return Err(match details {
            Some(details) => tr!(
                format!("MyMemory rejected the request: {details}"),
                format!("MyMemory відхилив запит: {details}")
            ),
            None => {
                let code = code.unwrap_or(u64::from(status));
                tr!(
                    format!("MyMemory returned status {code}."),
                    format!("MyMemory повернув статус {code}.")
                )
            }
        });
    }
    let text = reply.data.and_then(|data| data.text).unwrap_or_default();
    // Про вичерпаний ліміт MyMemory інколи повідомляє прямо в тексті перекладу.
    if text.contains("MYMEMORY WARNING")
        || text.contains("YOU USED ALL AVAILABLE FREE TRANSLATIONS")
    {
        return Err(quota().into());
    }
    if text.trim().is_empty() {
        return Err(tr!(
            "MyMemory returned an empty translation.",
            "MyMemory повернув порожній переклад."
        )
        .into());
    }
    Ok(text)
}

fn network_error(error: reqwest::Error) -> String {
    // Не додаємо URL або сирий текст помилки: вони можуть містити коментар.
    if error.is_timeout() {
        timed_out().into()
    } else {
        tr!(
            "Could not connect to MyMemory. Check the network.",
            "Не вдалося з’єднатися з MyMemory. Перевірте мережу."
        )
        .into()
    }
}

/// Ділить текст на шматки до `max` байтів. Розрив ставиться після переносу рядка,
/// потім після кінця речення, потім після пробілу, і лише за їх відсутності — посеред слова.
/// Шматки разом дають точно вхідний текст.
fn chunks(text: &str, max: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while rest.len() > max {
        let mut window = max;
        while !rest.is_char_boundary(window) {
            window -= 1;
        }
        if window == 0 {
            // Ліміт менший за один символ: беремо символ цілим, щоб цикл завершився.
            window = rest.chars().next().map_or(rest.len(), char::len_utf8);
        }
        let head = &rest[..window];
        let sentence = [". ", "! ", "? ", "; "]
            .iter()
            .filter_map(|end| head.rfind(end).map(|index| index + end.len()))
            .max();
        let cut = head
            .rfind('\n')
            .map(|index| index + 1)
            .or(sentence)
            .or_else(|| head.rfind(' ').map(|index| index + 1))
            .filter(|&cut| cut > 0)
            .unwrap_or(window);
        pieces.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        pieces.push(rest);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_uses_language_pair_and_optional_contact() {
        let query = |address: Url| -> std::collections::HashMap<String, String> {
            address.query_pairs().into_owned().collect()
        };
        let anonymous = query(url("Call x0_ & check.", "auto", "zh-Hant", None));
        assert_eq!(anonymous["q"], "Call x0_ & check.");
        assert_eq!(anonymous["langpair"], "autodetect|zh-TW");
        assert!(!anonymous.contains_key("de"));
        let query = query(url("Text", "EN", "uk-UA", Some("dev@example.com")));
        assert_eq!(query["langpair"], "en|uk");
        assert_eq!(query["de"], "dev@example.com");
    }

    #[test]
    fn decodes_translation_and_numeric_or_text_status() {
        for status in ["200", "\"200\""] {
            let body = format!(
                r#"{{"responseData":{{"translatedText":"Перший рядок.\nДругий."}},"responseStatus":{status},"quotaFinished":false}}"#
            );
            assert_eq!(
                decode(body.as_bytes(), 200).unwrap(),
                "Перший рядок.\nДругий."
            );
        }
    }

    #[test]
    fn reports_quota_limits_and_rejections() {
        let quota = r#"{"responseData":{"translatedText":"MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY"},"responseStatus":200}"#;
        assert!(decode(quota.as_bytes(), 200)
            .unwrap_err()
            .contains("mymemory_email"));
        let finished =
            r#"{"responseData":{"translatedText":"x"},"responseStatus":200,"quotaFinished":true}"#;
        assert!(decode(finished.as_bytes(), 200)
            .unwrap_err()
            .contains("quota"));
        let long = r#"{"responseData":{"translatedText":"QUERY LENGTH LIMIT EXCEEDED"},"responseStatus":403,"responseDetails":"QUERY LENGTH LIMIT EXCEEDED. MAX ALLOWED QUERY : 500 CHARS"}"#;
        assert!(decode(long.as_bytes(), 200)
            .unwrap_err()
            .contains("500 CHARS"));
        assert!(decode(&[], 429).unwrap_err().contains("429"));
        assert!(decode(b"<html>", 503).unwrap_err().contains("503"));
        for body in ["<html>", "{}", r#"{"responseStatus":200}"#] {
            assert!(decode(body.as_bytes(), 200).is_err(), "{body}");
        }
    }

    #[test]
    fn chunks_fit_the_limit_and_rejoin_losslessly() {
        let text = "First line.\nSecond sentence. Third one is longer than the rest!\nПерший рядок українською. Другий 😀 рядок.";
        for max in [12, 25, 40, 80] {
            let pieces = chunks(text, max);
            assert_eq!(pieces.concat(), text, "max {max}");
            assert!(pieces.iter().all(|piece| piece.len() <= max), "max {max}");
        }
        assert_eq!(chunks("short", 450), ["short"]);
        assert!(chunks("", 450).is_empty());
    }

    #[test]
    fn chunks_prefer_line_then_sentence_then_word_breaks() {
        assert_eq!(
            chunks("One two.\nThree four.", 15),
            ["One two.\n", "Three four."]
        );
        assert_eq!(
            chunks("One two. Three four five.", 15),
            ["One two. ", "Three four ", "five."]
        );
        assert_eq!(chunks("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        // Кирилиця займає два байти: розрив не потрапляє всередину символу.
        assert_eq!(chunks("абвгд", 5), ["аб", "вг", "д"]);
    }
}
