//! ChordPro 6.07 parser and AST.
//!
//! ChordPro (`https://www.chordpro.org`) is the long-standing standard for
//! chord-over-lyrics notation. This crate provides a comprehensive parser
//! and a typed AST aligned to **ChordPro 6.07** (May 2024).
//!
//! # Why not a third-party crate?
//!
//! No production-grade ChordPro parser exists in Rust at the time of
//! writing. The reference implementation is the Perl `chordpro` CLI; the
//! handful of crates on crates.io cover only `[Chord]Lyric` and a few
//! directives. This crate covers the full directive set, conditional
//! directives, `{define}` chord definitions, environments
//! (`{start_of_*}` / `{end_of_*}`), `[*annotation]` markers, line
//! continuation with `\`, `\uXXXX` escapes, and quoted directive
//! arguments.
//!
//! # Quick start
//!
//! ```
//! use keyflow_chordpro::{parse, Document, Directive};
//!
//! let src = "\
//! {title: Twinkle}
//! {artist: Traditional}
//!
//! [C]Twinkle, twinkle, [F]little [C]star
//! ";
//! let doc: Document = parse(src).expect("parse");
//! assert_eq!(doc.title(), Some("Twinkle"));
//! assert_eq!(doc.find_directive("artist").map(|d| d.value()), Some("Traditional"));
//! ```
//!
//! # Design
//!
//! - The AST is **lossless enough** for round-tripping: every directive,
//!   chunk, and annotation keeps a `source_span` so editors and the
//!   integrating LSP can map back to the original bytes.
//! - **Conditional directives** (`{title-en: Hello}`) are parsed as
//!   `Directive` with a `condition: Option<String>` field rather than
//!   forking the variant tree.
//! - **Unknown / custom** directives fall through to `Directive::Custom`
//!   so user `x_*` and forward-compat directives don't lose data.

mod ast;
mod parser;

#[cfg(test)]
mod tests;

pub use ast::{
    Annotation, ChordChunk, ChordDefinition, Directive, DirectiveKind, Document, Environment, Line,
    MetaItem, ParseError, ParseErrorKind, Section, Span,
};
pub use parser::{ParseOptions, parse, parse_with_options};
