//! Structured diagnostics for the IDE engine.
//!
//! Mirrors the shape of LSP's `Diagnostic` so the LSP server's translation
//! layer is trivial, while staying free of `lsp-types` so embedded callers
//! don't pull in protocol crates.

use crate::parsing::TextSpan;

/// Severity level for a diagnostic.
///
/// Values intentionally match LSP's `DiagnosticSeverity` numeric ordering
/// (Error = 1, Warning = 2, Information = 3, Hint = 4) so a future LSP layer
/// can `severity as u8` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Severity {
    Error = 1,
    Warning = 2,
    Info = 3,
    Hint = 4,
}

/// A structured parser / linter diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Byte range in the source document.
    pub range: TextSpan,
    /// Severity level.
    pub severity: Severity,
    /// Stable diagnostic code (e.g. `"kf001-unknown-chord"`). Used by users
    /// to suppress specific lints and by editors to link to docs.
    pub code: &'static str,
    /// Human-readable error message, free-form.
    pub message: String,
    /// Optional auto-fixes the editor can offer (Quick Fix in LSP terms).
    pub fixes: Vec<CodeAction>,
}

impl Diagnostic {
    pub fn new(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        range: TextSpan,
    ) -> Self {
        Self {
            range,
            severity,
            code,
            message: message.into(),
            fixes: Vec::new(),
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>, range: TextSpan) -> Self {
        Self::new(Severity::Error, code, message, range)
    }

    pub fn warning(code: &'static str, message: impl Into<String>, range: TextSpan) -> Self {
        Self::new(Severity::Warning, code, message, range)
    }

    pub fn hint(code: &'static str, message: impl Into<String>, range: TextSpan) -> Self {
        Self::new(Severity::Hint, code, message, range)
    }

    pub fn with_fix(mut self, fix: CodeAction) -> Self {
        self.fixes.push(fix);
        self
    }
}

/// A single textual edit that resolves a diagnostic.
///
/// Editors expose these as Quick Fixes / code actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeAction {
    pub title: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextSpan,
    pub new_text: String,
}

impl CodeAction {
    pub fn replace(title: impl Into<String>, range: TextSpan, new_text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            edits: vec![TextEdit {
                range,
                new_text: new_text.into(),
            }],
        }
    }
}
