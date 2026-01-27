//! Symbol model for code-level indexing.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Kind of code symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    /// Function.
    Function,
    /// Method.
    Method,
    /// Class/Struct.
    Class,
    /// Interface/Trait.
    Interface,
    /// Variable/Const.
    Variable,
    /// Enum.
    Enum,
    /// Module.
    Module,
    /// Other.
    Other,
}

/// A code symbol (function, class, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    /// Unique symbol ID.
    pub id: String,
    /// File ID this symbol belongs to.
    pub file_id: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Start line number (1-indexed).
    pub start_line: usize,
    /// End line number (1-indexed).
    pub end_line: usize,
    /// Doc comment if available.
    pub doc_comment: Option<String>,
    /// Symbol code snippet.
    pub code: String,
    /// Parent symbol ID (for nested symbols).
    pub parent_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_kind() {
        assert_eq!(SymbolKind::Function, SymbolKind::Function);
        assert_ne!(SymbolKind::Function, SymbolKind::Class);
    }

    #[test]
    fn test_symbol_new() {
        let symbol = Symbol {
            id: "sym-1".to_string(),
            file_id: "file-1".to_string(),
            name: "test_func".to_string(),
            kind: SymbolKind::Function,
            start_line: 10,
            end_line: 20,
            doc_comment: Some("Test function".to_string()),
            code: "fn test_func() {}".to_string(),
            parent_id: None,
        };

        assert_eq!(symbol.name, "test_func");
        assert_eq!(symbol.kind, SymbolKind::Function);
    }
}
