use reqwest::{header, Client, Url};
use serde_json::Value;
use std::{sync::OnceLock, time::Duration};

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/70.0.3538.102 Safari/537.36";
static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

#[derive(Debug)]
pub struct Translation {
    pub translation: String,
    pub source_language: String,
}

fn client() -> Result<&'static Client, String> {
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .referer(false)
                .build()
                .map_err(|_| "Не вдалося створити HTTP-клієнт Google Translate".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn url(text: &str, source: &str) -> Url {
    let mut url = Url::parse(ENDPOINT).expect("Некоректна вбудована адреса Google");
    // form_urlencoded зберігає буквальні +, &, # та Unicode у тексті коментаря.
    url.query_pairs_mut().extend_pairs([
        ("client", "gtx"),
        ("sl", source),
        ("tl", "uk"),
        ("dt", "t"),
        ("ie", "UTF-8"),
        ("oe", "UTF-8"),
        ("q", text),
    ]);
    url
}

fn decode(body: &[u8], status: u16, source: &str) -> Result<Translation, String> {
    if !(200..300).contains(&status) {
        return Err(match status {
            429 => {
                "Google Translate обмежив кількість запитів (HTTP 429). Спробуйте пізніше.".into()
            }
            403 => "Google Translate відхилив запит (HTTP 403).".into(),
            _ => format!("Google Translate повернув HTTP {status}."),
        });
    }
    let invalid = || "Некоректна відповідь Google Translate.".to_owned();
    let root: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let sentences = root
        .get(0)
        .and_then(Value::as_array)
        .filter(|sentences| !sentences.is_empty())
        .ok_or_else(invalid)?;
    let mut translated = String::new();
    for sentence in sentences {
        let first = sentence
            .as_array()
            .and_then(|parts| parts.first())
            .ok_or_else(invalid)?;
        if first.is_null() {
            continue;
        }
        translated.push_str(first.as_str().ok_or_else(invalid)?);
    }
    if translated.trim().is_empty() {
        return Err("Google Translate повернув порожній переклад.".into());
    }
    Ok(Translation {
        translation: translated,
        source_language: root
            .get(2)
            .and_then(Value::as_str)
            .unwrap_or(source)
            .to_owned(),
    })
}

pub async fn translate(
    text: &str,
    source: &str,
    deadline: Duration,
) -> Result<Translation, String> {
    translate_at(client()?, url(text, source), deadline, source).await
}

async fn translate_at(
    client: &Client,
    url: Url,
    deadline: Duration,
    source: &str,
) -> Result<Translation, String> {
    let operation = async {
        let response = request(client, url, deadline)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return decode(&[], status, source);
        }
        let bytes = response.bytes().await.map_err(network_error)?;
        decode(&bytes, status, source)
    };
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| "Час очікування Google Translate минув.".to_owned())?
}

fn request(client: &Client, url: Url, deadline: Duration) -> reqwest::RequestBuilder {
    client
        .get(url)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, USER_AGENT)
        .timeout(deadline)
}

fn network_error(error: reqwest::Error) -> String {
    // Не додаємо URL або сирий текст помилки: вони можуть містити коментар.
    if error.is_timeout() {
        "Час очікування Google Translate минув.".into()
    } else {
        "Не вдалося з’єднатися з Google Translate. Перевірте мережу.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_preserves_unicode_and_browser_headers() {
        let text = "C++ & ключ=value? #fragment\nДругий абзац.";
        let url = url(text, "auto");
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["q"], text);
        assert_eq!(query["client"], "gtx");
        assert_eq!(query["sl"], "auto");
        assert_eq!(query["tl"], "uk");
        assert!(!query.contains_key("key"));
        assert!(url.as_str().contains("C%2B%2B"));
        let request = request(client().unwrap(), url, Duration::from_secs(7))
            .build()
            .unwrap();
        assert_eq!(request.headers()[header::USER_AGENT], USER_AGENT);
    }

    #[test]
    fn decodes_sentences_and_paragraph_boundaries() {
        let body = r#"[[["Перший абзац.\n\n","First"],["Другий абзац.","Second"]],null,"en"]"#;
        let reply = decode(body.as_bytes(), 200, "auto").unwrap();
        assert_eq!(reply.translation, "Перший абзац.\n\nДругий абзац.");
        assert_eq!(reply.source_language, "en");
    }

    #[test]
    fn rejects_bad_responses_and_http_errors() {
        for body in [
            "<html>Blocked</html>",
            "{}",
            "[]",
            "[null]",
            "[[[123]]]",
            "[[[]]]",
            "[[[null]]]",
            r#"[[[" "]]]"#,
        ] {
            assert!(decode(body.as_bytes(), 200, "en").is_err(), "{body}");
        }
        for status in [403, 429, 500] {
            assert!(decode(&[], status, "en")
                .err()
                .unwrap()
                .contains(&status.to_string()));
        }
    }
    fn server(
        status: u16,
        body: &str,
        delay: Duration,
    ) -> (
        Url,
        std::thread::JoinHandle<String>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "http://{}/translate?q=private-comment",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let body = body.to_owned();
        listener.set_nonblocking(true).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            let stop = std::time::Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < stop =>
                    {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("Не вдалося прийняти тестовий HTTP-запит: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut input = Vec::new();
            let mut chunk = [0; 1024];
            while !input.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let n = socket.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                input.extend_from_slice(&chunk[..n]);
            }
            let _ = started.send(());
            std::thread::sleep(delay);
            let response = format!("HTTP/1.1 {status} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes());
            String::from_utf8(input).unwrap()
        });
        (url, thread, ready)
    }

    #[tokio::test]
    #[ignore = "Потрібен дозвіл на локальні TCP-сокети; запускається в Linux CI"]
    async fn http_transport_sends_headers_and_decodes_responses() {
        let client = Client::builder().no_proxy().build().unwrap();
        let (url, thread, _) = server(
            200,
            r#"[[["Переклад","Translation"]],null,"en"]"#,
            Duration::ZERO,
        );
        let reply = translate_at(&client, url, Duration::from_secs(3), "auto")
            .await
            .unwrap();
        assert_eq!(reply.translation, "Переклад");
        let request = thread.join().unwrap().to_lowercase();
        assert!(request.contains(&format!("user-agent: {}", USER_AGENT.to_lowercase())));
        assert!(request.contains("accept: application/json"));
        for status in [403, 429, 500] {
            let (url, thread, _) = server(status, "", Duration::ZERO);
            let error = translate_at(&client, url, Duration::from_secs(3), "en")
                .await
                .err()
                .unwrap();
            assert!(error.contains(&status.to_string()));
            assert!(!error.contains("private-comment"));
            thread.join().unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "Потрібен дозвіл на локальні TCP-сокети; запускається в Linux CI"]
    async fn http_transport_times_out_and_can_be_cancelled() {
        let client = Client::builder().no_proxy().build().unwrap();
        let (url, thread, _) = server(200, "[]", Duration::from_millis(100));
        let error = translate_at(&client, url, Duration::from_millis(30), "en")
            .await
            .err()
            .unwrap();
        assert!(error.contains("Час очікування"));
        thread.join().unwrap();

        let (url, thread, started) = server(200, "[]", Duration::from_millis(100));
        let task =
            tokio::spawn(
                async move { translate_at(&client, url, Duration::from_secs(3), "en").await },
            );
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        thread.join().unwrap();
    }
}
