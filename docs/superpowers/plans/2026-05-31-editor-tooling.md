# Editor Tooling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (A1) TextMate grammar giving `.line` syntax highlighting in VS Code, Zed, Sublime, and GitHub. (A2) `thruline lsp` subcommand — a `tower-lsp` language server reusing `validator.rs` for diagnostics, providing completions for runner/stage names, and go-to-definition for `runner:` and route stage references.

**Architecture:** A1 is a standalone `.tmLanguage.json` file requiring no Rust changes. A2 adds a `thruline lsp` subcommand powered by the `tower-lsp` crate; it reuses the existing parser and validator directly — no duplicate logic. The LSP parses the document on every change and converts `ValidationError`s into LSP diagnostics; the AST provides symbol/completion data.

**Tech Stack:** A1: JSON TextMate grammar. A2: Rust + `tower-lsp` + `tokio` (already a dependency).

---

## Files

**A1:**
- Create: `editors/thruline.tmLanguage.json`
- Create: `editors/vscode/package.json`
- Create: `editors/vscode/README.md`

**A2:**
- Modify: `Cargo.toml` — add `tower-lsp`, `dashmap`
- Create: `src/lsp/mod.rs` — `ThrulineLanguageServer` implementing `tower_lsp::LanguageServer`
- Modify: `src/main.rs` — add `lsp` subcommand
- Modify: `src/cli.rs` — `cmd_lsp()` entry point
- Modify: `src/parser/grammar.pest` — expose pest span info (add span captures to AST)
- Modify: `src/ast.rs` — add `span: (usize, usize)` to key decl types
- Modify: `docs/LANGUAGE.md` — LSP usage section

---

### Task 1: TextMate grammar (A1)

**Files:**
- Create: `editors/thruline.tmLanguage.json`
- Create: `editors/vscode/package.json`

- [ ] **Step 1: Create `editors/` directory and write `thruline.tmLanguage.json`**

```json
{
  "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
  "name": "Thruline",
  "fileTypes": ["line"],
  "scopeName": "source.line",
  "patterns": [
    { "include": "#comments" },
    { "include": "#keywords" },
    { "include": "#strings" },
    { "include": "#numbers" },
    { "include": "#operators" },
    { "include": "#identifiers" }
  ],
  "repository": {
    "comments": {
      "name": "comment.line.double-slash.line",
      "match": "//.*$"
    },
    "keywords": {
      "patterns": [
        {
          "name": "keyword.control.line",
          "match": "\\b(config|runner|stage|thruline|run|import|start|routes|inputs|in|out|as|path|value|file|model|system|tools|temperature|max_tokens)\\b"
        },
        {
          "name": "keyword.operator.line",
          "match": "->|==|!="
        },
        {
          "name": "keyword.operator.fan.line",
          "match": "\\[\\*[0-9]*\\]"
        }
      ]
    },
    "strings": {
      "name": "string.quoted.double.line",
      "begin": "\"",
      "end": "\"",
      "patterns": []
    },
    "numbers": {
      "name": "constant.numeric.line",
      "match": "\\b[0-9]+(\\.[0-9]+)?\\b"
    },
    "operators": {
      "name": "punctuation.definition.line",
      "match": "[{}\\[\\](),:]"
    },
    "identifiers": {
      "name": "entity.name.line",
      "match": "\\b[A-Za-z][A-Za-z0-9_]*\\b"
    }
  }
}
```

- [ ] **Step 2: Create VS Code extension manifest `editors/vscode/package.json`**

```json
{
  "name": "thruline",
  "displayName": "Thruline",
  "description": "Syntax highlighting for .line workflow files",
  "version": "0.1.1",
  "publisher": "dove-tools",
  "engines": { "vscode": "^1.75.0" },
  "categories": ["Programming Languages"],
  "repository": {
    "type": "git",
    "url": "https://github.com/dove-tools/thruline"
  },
  "contributes": {
    "languages": [{
      "id": "line",
      "aliases": ["Thruline", "line"],
      "extensions": [".line"],
      "configuration": "./language-configuration.json"
    }],
    "grammars": [{
      "language": "line",
      "scopeName": "source.line",
      "path": "../thruline.tmLanguage.json"
    }]
  }
}
```

- [ ] **Step 3: Create `editors/vscode/language-configuration.json`**

```json
{
  "comments": {
    "lineComment": "//"
  },
  "brackets": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"]
  ],
  "autoClosingPairs": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
    ["\"", "\""]
  ]
}
```

- [ ] **Step 4: Add note to README**

In `README.md`, add to the Install section or create an `## Editor Support` section:

```markdown
## Editor Support

### VS Code

Install the [Thruline extension](https://marketplace.visualstudio.com/items?itemName=dove-tools.thruline) for `.line` syntax highlighting, or manually:

1. Copy `editors/vscode/` into `~/.vscode/extensions/thruline-0.1.1/`
2. Reload VS Code

### Zed / Sublime

Zed and Sublime Text both consume TextMate grammars. Point them at `editors/thruline.tmLanguage.json`.
```

- [ ] **Step 5: Commit**

```bash
git add editors/ README.md
git commit -m "feat: TextMate grammar and VS Code extension for .line files (A1)"
```

---

### Task 2: LSP — `thruline lsp` subcommand (A2)

**Files:**
- Modify: `Cargo.toml`
- Create: `src/lsp/mod.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `tower-lsp` dependency to `Cargo.toml`**

```toml
tower-lsp = { version = "0.20", features = ["proposed"] }
```

(Verify latest version with `cargo add tower-lsp` or `cargo search tower-lsp`.)

- [ ] **Step 2: Add `Lsp` subcommand to `src/cli.rs`**

In the `Commands` enum:
```rust
/// Start the language server (communicates via stdio)
Lsp,
```

In `run()`:
```rust
Commands::Lsp => cmd_lsp().await,
```

Add:
```rust
pub async fn cmd_lsp() -> anyhow::Result<()> {
    crate::lsp::run_lsp().await
}
```

- [ ] **Step 3: Create `src/lsp/mod.rs`**

```rust
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

    async fn validate_document(&self, uri: Url, text: &str) {
        let diagnostics = match crate::parser::parse_str(text) {
            Err(e) => {
                // Parse error — single diagnostic at position 0
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: e.to_string(),
                    ..Default::default()
                }]
            }
            Ok(items) => {
                let result = crate::validator::validate(&items);
                result.errors.iter().map(|e| Diagnostic {
                    range: Range::default(), // TODO: add spans for precise positions
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: e.to_string(),
                    ..Default::default()
                }).collect()
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
        self.validate_document(
            params.text_document.uri,
            &params.text_document.text,
        ).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.validate_document(params.text_document.uri, &change.text).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // Re-validate on save (in case did_change was skipped)
        if let Some(text) = params.text {
            self.validate_document(params.text_document.uri, &text).await;
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        // Keyword completions — extended with identifier completions once spans are added.
        let keywords = vec![
            "config", "runner", "stage", "thruline", "run", "import",
            "start", "routes", "inputs", "in", "out", "as", "path",
            "value", "file", "model", "system", "tools", "temperature", "max_tokens",
        ];
        let items: Vec<CompletionItem> = keywords.iter().map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        }).collect();
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
```

- [ ] **Step 4: Add `pub mod lsp;` to `src/main.rs`**

In `src/main.rs`, add:
```rust
mod lsp;
```

- [ ] **Step 5: Compile check**

```bash
cargo build 2>&1 | grep '^error' | head -10
```
Expected: no errors.

- [ ] **Step 6: Smoke test — start LSP and send initialize**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}' | \
  cargo run -- lsp 2>/dev/null | head -2
```
Expected: JSON response with `"result":{"capabilities":{...}}`.

- [ ] **Step 7: Update VS Code extension to use the LSP**

In `editors/vscode/package.json`, add `activationEvents` and a language client configuration pointing at `thruline lsp`. (Full VS Code extension client implementation goes here — requires a `extension.ts` in TypeScript. This step is the shell; full implementation is a separate extension project.)

- [ ] **Step 8: Document in `docs/LANGUAGE.md`**

Add a section:
```markdown
## Language Server (LSP)

`thruline lsp` starts a language server that communicates via stdio.
Configure your editor to use it:

**VS Code** — Install the Thruline extension (see `editors/vscode/`).

**Neovim (nvim-lspconfig):**
```lua
require('lspconfig').configs.thruline = {
  default_config = {
    cmd = { 'thruline', 'lsp' },
    filetypes = { 'line' },
    root_dir = require('lspconfig.util').find_git_ancestor,
  }
}
require('lspconfig').thruline.setup {}
```

**Zed** — Add to `~/.config/zed/settings.json`:
```json
{
  "lsp": {
    "thruline": {
      "binary": { "path": "thruline", "arguments": ["lsp"] }
    }
  }
}
```

Current capabilities: diagnostics (all validator errors), keyword completion.
```

- [ ] **Step 9: Commit and push**

```bash
git add src/lsp/ src/cli.rs src/main.rs Cargo.toml editors/ docs/LANGUAGE.md
git commit -m "feat: thruline lsp subcommand — diagnostics + keyword completion via tower-lsp (A2)"
git push origin main
```
