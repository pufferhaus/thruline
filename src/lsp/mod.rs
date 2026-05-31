use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

pub struct ThrulineLanguageServer {
    client: Client,
}

impl ThrulineLanguageServer {
    fn new(client: Client) -> Self {
        Self { client }
    }

    async fn validate_and_publish(&self, uri: Url, text: &str) {
        let diagnostics = match crate::parser::parse_str(text) {
            Err(e) => vec![Diagnostic {
                range: Range::default(),
                severity: Some(DiagnosticSeverity::ERROR),
                message: e.to_string(),
                ..Default::default()
            }],
            Ok(items) => {
                let result = crate::validator::validate(&items);
                let errors = result.errors.iter().map(|e| Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: e.to_string(),
                    ..Default::default()
                });
                let warnings = result.warnings.iter().map(|w| Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::WARNING),
                    message: w.0.clone(),
                    ..Default::default()
                });
                errors.chain(warnings).collect()
            }
        };
        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for ThrulineLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![" ".to_string(), ":".to_string()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "thruline-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.validate_and_publish(params.text_document.uri, &params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.validate_and_publish(params.text_document.uri, &change.text)
                .await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = params.text {
            self.validate_and_publish(params.text_document.uri, &text).await;
        }
    }

    async fn hover(&self, _params: HoverParams) -> Result<Option<Hover>> {
        Ok(None) // Placeholder — span info needed for full hover
    }

    async fn completion(
        &self,
        _params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let keywords = [
            "config",
            "runner",
            "stage",
            "thruline",
            "run",
            "import",
            "start",
            "routes",
            "inputs",
            "in",
            "out",
            "as",
            "path",
            "value",
            "file",
            "model",
            "system",
            "tools",
            "temperature",
            "max_tokens",
        ];
        let items: Vec<CompletionItem> = keywords
            .iter()
            .map(|kw| CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            })
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }
}

pub async fn run_lsp() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(ThrulineLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}
