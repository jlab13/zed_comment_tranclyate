mod comments;
mod google;
#[cfg(all(target_os = "macos", feature = "apple"))]
mod native;
mod text;
mod translate;

use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Notify, RwLock};
use tower::Service;
use tower_lsp::{jsonrpc, lsp_types::*, Client, LanguageServer, LspService, Server};
use translate::{Config, Translator};

struct Backend {
    client: Client,
    documents: RwLock<HashMap<Url, (i32, String)>>,
    translator: RwLock<Arc<Translator>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        let config: Config = params
            .initialization_options
            .map(serde_json::from_value)
            .transpose()
            .map_err(|err| jsonrpc::Error::invalid_params(err.to_string()))?
            .unwrap_or_default();
        config.validate().map_err(jsonrpc::Error::invalid_params)?;
        *self.translator.write().await = Arc::new(Translator::new(config));
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
        if doc.language_id == "c" {
            self.documents
                .write()
                .await
                .insert(doc.uri, (doc.version, doc.text));
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut documents = self.documents.write().await;
        if let Some((version, text)) = documents.get_mut(&params.text_document.uri) {
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
        let (version, comment) = {
            let documents = self.documents.read().await;
            let Some((version, source)) = documents.get(&target.text_document.uri) else {
                return Ok(None);
            };
            let Some(comment) = comments::at(source, target.position) else {
                return Ok(None);
            };
            (*version, comment)
        };
        let translator = self.translator.read().await.clone();
        let value = match translator.translate(&comment.text).await {
            Ok(text) => format!("Переклад українською\n\n{text}"),
            Err(error) => {
                self.client.log_message(MessageType::WARNING, &error).await;
                format!("Переклад недоступний\n\n{error}")
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
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value,
            }),
            range: Some(comment.range),
        }))
    }
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

#[cfg(all(target_os = "macos", feature = "apple"))]
fn main() {
    std::thread::Builder::new()
        .name("comment-translate-lsp".into())
        .spawn(run_server)
        .expect("Не вдалося запустити потік LSP");
    native::run_main_loop();
}

#[cfg(not(all(target_os = "macos", feature = "apple")))]
fn main() {
    run_server();
}

fn run_server() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Не вдалося запустити середовище LSP");
    let code = runtime.block_on(async {
        if let Some(code) = diagnostics().await {
            return code;
        }
        let (service, socket) = LspService::new(|client| Backend {
            client,
            documents: RwLock::new(HashMap::new()),
            translator: RwLock::new(Arc::new(Translator::new(Config::default()))),
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
        "--check" if args.len() <= 2 => serde_json::json!({
            "operation":"check",
            "source_language":args.get(1).map(String::as_str).unwrap_or("en"),
            "timeout_seconds":15
        })
        .to_string(),
        "--translate" if args.len() == 1 => {
            let mut input = String::new();
            if std::io::stdin().read_to_string(&mut input).is_err() {
                return Some(1);
            }
            input
        }
        _ => {
            eprintln!("Використання: comment-translate-lsp [--check en | --translate]");
            return Some(1);
        }
    };
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
