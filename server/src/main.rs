mod comments;
mod deepl;
mod google;
mod i18n;
mod languages;
mod locale;
mod mymemory;
mod segments;
mod text;
mod translate;

use i18n::tr;
use languages::LanguageSpec;
use locale::TargetLanguage;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Notify, RwLock};
use tower::Service;
use tower_lsp::{jsonrpc, lsp_types::*, Client, LanguageServer, LspService, Server};
use translate::{Config, Translator};

struct Backend {
    client: Client,
    /// Версія, текст і мова кожного відкритого документа підтримуваної мови.
    documents: RwLock<HashMap<Url, (i32, String, &'static LanguageSpec)>>,
    translator: RwLock<Arc<Translator>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        // Мова повідомлень потрібна ще до перевірки, щоб і помилки налаштувань були зрозумілі.
        i18n::set_for_target(&translate::requested_target(
            params.initialization_options.as_ref(),
        ));
        let config: Config = params
            .initialization_options
            .map(serde_json::from_value)
            .transpose()
            .map_err(|err| jsonrpc::Error::invalid_params(err.to_string()))?
            .unwrap_or_default();
        config.validate().map_err(jsonrpc::Error::invalid_params)?;
        let target = config.target_language(locale::system_language);
        i18n::set_for_target(&target.code);
        self.client
            .log_message(
                MessageType::INFO,
                tr!(
                    format!("Target language: {} ({})", target.code, target.origin),
                    format!("Мова перекладу: {} ({})", target.code, target.origin)
                ),
            )
            .await;
        let chain: Vec<_> = config.chain().into_iter().map(|p| p.name()).collect();
        let message = if chain.is_empty() {
            tr!(
                "No translation provider selected: comments are not sent anywhere until you set provider.".to_owned(),
                "Провайдера перекладу не вибрано: коментарі нікуди не надсилаються, доки не задано provider.".to_owned()
            )
        } else {
            tr!(
                format!("Translation order: {}", chain.join(" → ")),
                format!("Порядок перекладу: {}", chain.join(" → "))
            )
        };
        self.client.log_message(MessageType::INFO, message).await;
        // Сам ключ ніколи не потрапляє в журнал: лише план або причина, чому DeepL пропускається.
        match config.deepl_key() {
            Some(Ok(key)) => {
                let plan = deepl::plan(&key);
                let message = tr!(
                    format!("DeepL: key found, {plan} plan"),
                    format!("DeepL: ключ знайдено, план {plan}")
                );
                self.client.log_message(MessageType::INFO, message).await;
            }
            Some(Err(reason)) => {
                let message = tr!(
                    format!("{reason} DeepL is skipped."),
                    format!("{reason} DeepL пропускається.")
                );
                self.client.log_message(MessageType::WARNING, message).await;
            }
            None => {}
        }
        *self.translator.write().await = Arc::new(Translator::new(config, target));
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "comment-translate-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        let mut documents = self.documents.write().await;
        // Документи непідтримуваних мов не зберігаємо, тому hover для них не з’являється.
        match languages::resolve(&doc.language_id, &doc.uri) {
            Some(language) => {
                documents.insert(doc.uri, (doc.version, doc.text, language));
            }
            None => {
                documents.remove(&doc.uri);
            }
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut documents = self.documents.write().await;
        if let Some((version, text, _)) = documents.get_mut(&params.text_document.uri) {
            if params.text_document.version <= *version {
                return;
            }
            for change in params.content_changes {
                // Сервер запитує повні зміни документа, без діапазонів.
                if change.range.is_none() {
                    *text = change.text;
                }
            }
            *version = params.text_document.version;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> jsonrpc::Result<Option<Hover>> {
        let target = params.text_document_position_params;
        let (version, comment, markdown) = {
            let documents = self.documents.read().await;
            let Some((version, source, language)) = documents.get(&target.text_document.uri) else {
                return Ok(None);
            };
            let Some(comment) = comments::at(source, target.position, language) else {
                return Ok(None);
            };
            // Документацію Rust, Swift і Dart пишуть у Markdown; звичайні коментарі — ні.
            let markdown = language.markdown_fence.filter(|_| comment.doc);
            (*version, comment, markdown)
        };
        let translator = self.translator.read().await.clone();
        if !translator.configured() {
            // Без вибраного сервісу текст коментаря нікуди не йде: лише пояснюємо, як його вибрати.
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: setup_hint(),
                }),
                range: Some(comment.range),
            }));
        }
        let (kind, value) = match translator.translate(&comment.text).await {
            Ok(text) => {
                let code = &translator.target().code;
                let header = tr!(
                    format!("Translation → {code}"),
                    format!("Переклад → {code}")
                );
                match markdown {
                    Some(fence) => (
                        MarkupKind::Markdown,
                        format!("{header}\n\n{}", text::markdown(&text, fence)),
                    ),
                    None => (MarkupKind::PlainText, format!("{header}\n\n{text}")),
                }
            }
            Err(error) => {
                self.client.log_message(MessageType::WARNING, &error).await;
                (
                    MarkupKind::PlainText,
                    tr!(
                        format!("Translation unavailable\n\n{error}"),
                        format!("Переклад недоступний\n\n{error}")
                    ),
                )
            }
        };
        // Не показуємо переклад зі старими координатами після редагування.
        if self
            .documents
            .read()
            .await
            .get(&target.text_document.uri)
            .map(|doc| doc.0)
            != Some(version)
        {
            return Ok(None);
        }
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent { kind, value }),
            range: Some(comment.range),
        }))
    }
}

/// Підказка замість перекладу, доки користувач не вибрав сервіс перекладу.
fn setup_hint() -> String {
    let example = "```json\n\"lsp\": {\n  \"comment-translate\": {\n    \"initialization_options\": { \"provider\": \"google\" }\n  }\n}\n```";
    tr!(
        format!(
            "**Comment Translation: choose a translation provider**\n\n\
             Comments are not sent anywhere until you choose a provider in your Zed settings, \
             for example:\n\n{example}\n\n\
             - `google` — Google Translate, no key\n\
             - `mymemory` — MyMemory, no key, daily limit\n\
             - `deepl` — DeepL with your own API key (`deepl_key_file` or `DEEPL_AUTH_KEY`)\n\n\
             The chosen service then receives the comment text. Google and DeepL fall back to \
             other services by default; `\"fallback\": \"none\"` keeps only the chosen one. \
             After changing the settings, run `editor: restart language server`."
        ),
        format!(
            "**Comment Translation: виберіть сервіс перекладу**\n\n\
             Коментарі нікуди не надсилаються, доки ви не виберете провайдера в налаштуваннях \
             Zed, наприклад:\n\n{example}\n\n\
             - `google` — Google Translate, без ключа\n\
             - `mymemory` — MyMemory, без ключа, денний ліміт\n\
             - `deepl` — DeepL з власним ключем API (`deepl_key_file` або `DEEPL_AUTH_KEY`)\n\n\
             Після цього текст коментаря отримуватиме вибраний сервіс. Google і DeepL за \
             замовчуванням переходять на інші сервіси; `\"fallback\": \"none\"` залишає лише \
             вибраний. Після зміни налаштувань виконайте `editor: restart language server`."
        )
    )
}

struct ExitAware<S> {
    inner: S,
    exit: Arc<Notify>,
}

impl<S: Service<jsonrpc::Request>> Service<jsonrpc::Request> for ExitAware<S> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: jsonrpc::Request) -> Self::Future {
        let exiting = request.method() == "exit";
        let future = self.inner.call(request);
        if exiting {
            self.exit.notify_one();
        }
        future
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Could not start the LSP runtime");
    let code = runtime.block_on(async {
        if let Some(code) = diagnostics().await {
            return code;
        }
        let (service, socket) = LspService::new(|client| Backend {
            client,
            documents: RwLock::new(HashMap::new()),
            translator: RwLock::new(Arc::new(Translator::new(
                Config::default(),
                TargetLanguage::fallback(),
            ))),
        });
        let exit = Arc::new(Notify::new());
        let service = ExitAware {
            inner: service,
            exit: exit.clone(),
        };
        // Транспорт tower-lsp може чекати наступний ввід навіть після exit.
        tokio::select! {
            _ = Server::new(tokio::io::stdin(), tokio::io::stdout(), socket).serve(service) => {},
            _ = exit.notified() => {},
        }
        0
    });
    // Після LSP exit не чекаємо блокувального читання stdin до закриття каналу клієнтом.
    runtime.shutdown_background();
    std::process::exit(code);
}

async fn diagnostics() -> Option<i32> {
    use std::io::Read;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return None;
    }
    let input = match args[0].as_str() {
        "--check" if args.len() <= 3 => {
            let mut request = serde_json::json!({
                "operation":"check",
                "source_language":args.get(1).map(String::as_str).unwrap_or("en"),
                "timeout_seconds":15
            });
            if let Some(target) = args.get(2) {
                request["target_language"] = target.as_str().into();
            }
            request.to_string()
        }
        "--translate" if args.len() == 1 => {
            let mut input = String::new();
            if std::io::stdin().read_to_string(&mut input).is_err() {
                return Some(1);
            }
            input
        }
        _ => {
            eprintln!("Usage: comment-translate-lsp [--check en [uk] | --translate]");
            return Some(1);
        }
    };
    // CLI теж відповідає мовою перекладу: українською для uk, інакше англійською.
    let request = serde_json::from_str::<serde_json::Value>(&input).ok();
    i18n::set_for_target(&translate::requested_target(request.as_ref()));
    let (json, code) = match translate::diagnose(&input).await {
        Ok(value) => {
            let code = i32::from(!value["error"].is_null());
            (value.to_string(), code)
        }
        Err(error) => (serde_json::json!({"error":error}).to_string(), 1),
    };
    println!("{json}");
    Some(code)
}
