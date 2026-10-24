use std::{
    collections::HashMap,
    ffi::{c_char, CStr, CString},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};
use tokio::sync::oneshot;

type Pending = HashMap<u64, oneshot::Sender<String>>;
static PENDING: OnceLock<Mutex<Pending>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

extern "C" {
    fn comment_translate_start(
        id: u64,
        json: *const c_char,
        callback: extern "C" fn(u64, *const c_char),
    );
    fn comment_translate_cancel(id: u64);
    fn comment_translate_run_main_loop() -> !;
}

pub fn run_main_loop() -> ! {
    // Викликається лише з головного потоку процесу; LSP працює на потоці Rust.
    unsafe { comment_translate_run_main_loop() }
}

fn pending() -> &'static Mutex<Pending> {
    PENDING.get_or_init(Mutex::default)
}

extern "C" fn complete(id: u64, response: *const c_char) {
    if response.is_null() {
        return;
    }
    // Swift гарантує дійсний C-рядок до завершення callback. Вказівник не зберігаємо.
    let response = unsafe { CStr::from_ptr(response) }
        .to_string_lossy()
        .into_owned();
    if let Some(sender) = pending()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&id)
    {
        let _ = sender.send(response);
    }
}

struct RequestGuard(u64);

impl Drop for RequestGuard {
    fn drop(&mut self) {
        pending()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.0);
        // Після видалення ідентифікатора пізній callback ігнорується.
        unsafe { comment_translate_cancel(self.0) };
    }
}

pub async fn request(json: String, deadline: Duration) -> Result<String, String> {
    let json = CString::new(json).map_err(|_| "Запит містить нульовий байт")?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = oneshot::channel();
    pending()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(id, sender);
    let _guard = RequestGuard(id);
    // Swift копіює json синхронно; callback використовує лише числовий ідентифікатор.
    unsafe { comment_translate_start(id, json.as_ptr(), complete) };
    tokio::time::timeout(deadline, receiver)
        .await
        .map_err(|_| "Час очікування перекладу минув")?
        .map_err(|_| "Бібліотека перекладу завершила запит без відповіді".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_bridge_preserves_unicode_and_embedded_null() {
        let text = "Український текст 😀\u{0} другий рядок\n";
        let input = serde_json::json!({"text":text,"source_language":"uk"}).to_string();
        let reply = request(input, Duration::from_secs(3)).await.unwrap();
        let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["translation"], text);
    }

    #[tokio::test]
    async fn static_bridge_reports_invalid_json_without_terminating_lsp() {
        let reply = request("{".into(), Duration::from_secs(3)).await.unwrap();
        assert!(reply.contains("JSON"));
        let reply = request(
            r#"{"operation":"check","source_language":"uk"}"#.into(),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert!(reply.contains("same_language"));
    }

    #[test]
    fn late_callback_after_cancellation_is_safe() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (sender, mut receiver) = oneshot::channel();
        pending().lock().unwrap().insert(id, sender);
        drop(RequestGuard(id));
        let response = CString::new(r#"{"translation":"Пізня відповідь"}"#).unwrap();
        complete(id, response.as_ptr());
        assert!(receiver.try_recv().is_err());
        assert!(!pending().lock().unwrap().contains_key(&id));
    }
}
