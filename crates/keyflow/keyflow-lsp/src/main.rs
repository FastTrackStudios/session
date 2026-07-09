//! Keyflow Language Server.
//!
//! Wraps the `keyflow_text::ide` engine in a `tower-lsp` server. Speaks LSP
//! over stdio, so any editor that supports LSP (VS Code, Zed, Helix,
//! Neovim, Sublime LSP, Emacs `eglot` / `lsp-mode`) can light up keyflow
//! files without any editor-specific code.
//!
//! ## Architecture
//!
//! The server stores per-document state in a `DashMap<Url, String>` and
//! re-runs `analyze` on every change. For typical chart sizes this is
//! sub-millisecond. When charts grow large enough to matter, the engine can
//! be swapped for an incremental implementation without touching the LSP
//! glue.

use std::collections::HashMap;

use dashmap::DashMap;
use keyflow_syntax::highlighting::HighlightKind;
use keyflow_text::ide::{
    self, CompletionKind, Diagnostic as IdeDiagnostic, Severity as IdeSeverity,
};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        // Stderr — stdout is the LSP transport.
        .with_writer(std::io::stderr)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: DashMap::new(),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

struct Backend {
    client: Client,
    documents: DashMap<Url, String>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "keyflow-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "$".to_string(),
                        "/".to_string(),
                        " ".to_string(),
                        "|".to_string(),
                    ]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: SEMANTIC_TOKEN_TYPES
                                    .iter()
                                    .map(|t| SemanticTokenType::new(t))
                                    .collect(),
                                token_modifiers: vec![],
                            },
                            range: Some(false),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..Default::default()
                        },
                    ),
                ),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "keyflow-lsp ready")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        self.documents.insert(uri.clone(), text.clone());
        self.publish_diagnostics(uri, &text).await;
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        // We declared FULL sync, so each change replaces the whole document.
        if let Some(change) = params.content_changes.pop() {
            self.documents.insert(uri.clone(), change.text.clone());
            self.publish_diagnostics(uri, &change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.remove(&uri);
        // Clear diagnostics on the client side.
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let Some(text) = self.documents.get(&uri).map(|d| d.clone()) else {
            return Ok(None);
        };
        let offset = ide::line_col_to_offset(&text, pos.line, pos.character);
        let analysis = ide::analyze(&text);
        let items: Vec<CompletionItem> = ide::complete(&text, offset, &analysis.chart)
            .into_iter()
            .map(to_lsp_completion)
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(text) = self.documents.get(&uri).map(|d| d.clone()) else {
            return Ok(None);
        };
        let offset = ide::line_col_to_offset(&text, pos.line, pos.character);
        let analysis = ide::analyze(&text);
        let Some(info) = ide::hover(&text, offset, &analysis.chart) else {
            return Ok(None);
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info.markdown,
            }),
            range: Some(span_to_range(&text, info.range)),
        }))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let Some(text) = self.documents.get(&uri).map(|d| d.clone()) else {
            return Ok(None);
        };
        let analysis = ide::analyze(&text);
        let data = encode_semantic_tokens(&text, &analysis.highlights);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

impl Backend {
    async fn publish_diagnostics(&self, uri: Url, text: &str) {
        let analysis = ide::analyze(text);
        let diags: Vec<Diagnostic> = analysis
            .diagnostics
            .iter()
            .map(|d| ide_diag_to_lsp(text, d))
            .collect();
        self.client.publish_diagnostics(uri, diags, None).await;
    }
}

fn ide_diag_to_lsp(text: &str, d: &IdeDiagnostic) -> Diagnostic {
    Diagnostic {
        range: span_to_range(text, d.range),
        severity: Some(match d.severity {
            IdeSeverity::Error => DiagnosticSeverity::ERROR,
            IdeSeverity::Warning => DiagnosticSeverity::WARNING,
            IdeSeverity::Info => DiagnosticSeverity::INFORMATION,
            IdeSeverity::Hint => DiagnosticSeverity::HINT,
        }),
        code: Some(NumberOrString::String(d.code.to_string())),
        code_description: None,
        source: Some("keyflow".to_string()),
        message: d.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

fn span_to_range(text: &str, span: keyflow_syntax::parsing::TextSpan) -> Range {
    let (sl, sc) = ide::offset_to_line_col(text, span.start);
    let (el, ec) = ide::offset_to_line_col(text, span.end());
    Range {
        start: Position::new(sl, sc),
        end: Position::new(el, ec),
    }
}

fn to_lsp_completion(c: ide::Completion) -> CompletionItem {
    CompletionItem {
        label: c.label.clone(),
        kind: Some(match c.kind {
            CompletionKind::Chord => CompletionItemKind::VALUE,
            CompletionKind::Quality => CompletionItemKind::ENUM_MEMBER,
            CompletionKind::Section => CompletionItemKind::CLASS,
            CompletionKind::Command => CompletionItemKind::FUNCTION,
            CompletionKind::MelodyVar => CompletionItemKind::VARIABLE,
            CompletionKind::Keyword => CompletionItemKind::KEYWORD,
            CompletionKind::Snippet => CompletionItemKind::SNIPPET,
        }),
        detail: c.detail,
        insert_text: c.insert_text,
        ..Default::default()
    }
}

// ---------------- Semantic-tokens encoding ----------------

const SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "keyword",  // 0  Section, Command
    "variable", // 1  Root, ScaleDegree, RomanNumeral, MelodyVar
    "operator", // 2  Accidental, Modifier
    "type",     // 3  Quality
    "number",   // 4  Extension, Tempo, MeasureCount
    "string",   // 5  Comment, TextCue, DynamicMarking
    "function", // 6  RhythmSlash, MelodyNote
    "property", // 7  Bass, Inversion
];

fn token_type_for(kind: HighlightKind) -> u32 {
    use HighlightKind::*;
    match kind {
        // variable
        Root | ScaleDegree | RomanNumeral | MemoryRecall => 1,
        // operator
        Accidental | Modifier | Push | Pull | Triplet | Dot | SlashRhythm | MeasureSeparator
        | SectionBracket | Repeat | TempoArrow => 2,
        // type
        Quality => 3,
        // number
        Extension | Tempo | MeasureCount | Duration => 4,
        // string
        Comment | CommentMarker | TextCue | Dynamic | Title | Artist => 5,
        // keyword
        Section | SectionComment | TrackMarker | Command | Key | TimeSignature => 0,
        // function
        Rest | Space | MelodyBlock => 6,
        // property
        Bass | BassSlash => 7,
        // fallback (Whitespace, Unknown)
        Whitespace | Unknown => 1,
    }
}

/// Encode line-relative semantic-tokens deltas as the LSP wants them.
fn encode_semantic_tokens(
    text: &str,
    highlights: &[keyflow_syntax::highlighting::HighlightSpan],
) -> Vec<SemanticToken> {
    // Sort by start offset.
    let mut spans: Vec<&keyflow_syntax::highlighting::HighlightSpan> = highlights.iter().collect();
    spans.sort_by_key(|s| s.span.start);

    // Cache a mapping from byte offset to (line, col) by precomputing line starts.
    let line_starts = build_line_starts(text);
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;
    let mut out = Vec::with_capacity(spans.len());

    for s in spans {
        let (line, col) = byte_to_line_col(&line_starts, s.span.start);
        // LSP only supports single-line tokens — clamp to end-of-line if a
        // span happened to wrap (none of our highlighter spans do today).
        let line_end = line_starts
            .get((line + 1) as usize)
            .copied()
            .unwrap_or(text.len());
        let len = (s.span.start + s.span.len).min(line_end) - s.span.start;
        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 { col - prev_col } else { col };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: len as u32,
            token_type: token_type_for(s.kind),
            token_modifiers_bitset: 0,
        });
        prev_line = line;
        prev_col = col;
    }

    out
}

fn build_line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0usize];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            v.push(i + 1);
        }
    }
    v
}

fn byte_to_line_col(line_starts: &[usize], offset: usize) -> (u32, u32) {
    let line = match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let col = offset - line_starts[line];
    (line as u32, col as u32)
}

#[allow(dead_code)]
fn _silence_unused_imports(_: HashMap<(), ()>) {}
