//! Rust binding for the [tree-sitter-keyflow](https://crates.io/crates/tree-sitter-keyflow) grammar.
//!
//! ```ignore
//! use tree_sitter::Parser;
//! let mut parser = Parser::new();
//! parser
//!     .set_language(&tree_sitter_keyflow::LANGUAGE.into())
//!     .expect("load keyflow grammar");
//! let tree = parser.parse("VS 1: | 1 4 5 1 |\n", None).unwrap();
//! assert!(tree.root_node().child_count() > 0);
//! ```

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_keyflow() -> *const ();
}

/// The grammar's `LanguageFn`. Pass `&LANGUAGE.into()` to
/// [`tree_sitter::Parser::set_language`] / [`tree_sitter::Query::new`].
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_keyflow) };

/// Tree-sitter highlight query bundled with the grammar.
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");

/// Tree-sitter injection query bundled with the grammar.
pub const INJECTIONS_QUERY: &str = include_str!("../../queries/injections.scm");

/// Tree-sitter local-scope query bundled with the grammar.
pub const LOCALS_QUERY: &str = include_str!("../../queries/locals.scm");

/// Recommended file extensions for editor integrations.
pub const FILE_TYPES: &[&str] = &["kf", "keyflow"];

#[cfg(test)]
mod tests {
    #[test]
    fn highlight_queries_are_non_empty() {
        assert!(!super::HIGHLIGHTS_QUERY.is_empty());
        assert!(!super::INJECTIONS_QUERY.is_empty());
        assert!(!super::LOCALS_QUERY.is_empty());
    }
}
