//! Symbol model for code-level indexing.

use serde::{Deserialize, Serialize};
use std::fmt;

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

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "Function"),
            SymbolKind::Method => write!(f, "Method"),
            SymbolKind::Class => write!(f, "Class"),
            SymbolKind::Interface => write!(f, "Interface"),
            SymbolKind::Variable => write!(f, "Variable"),
            SymbolKind::Enum => write!(f, "Enum"),
            SymbolKind::Module => write!(f, "Module"),
            SymbolKind::Other => write!(f, "Other"),
        }
    }
}

/// A code symbol (function, class, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Git branch name where this symbol was extracted.
    pub branch_name: String,
    /// Last commit hash that modified this symbol.
    pub last_commit_hash: Option<String>,
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
            branch_name: "main".to_string(),
            last_commit_hash: Some("abc123".to_string()),
        };

        assert_eq!(symbol.name, "test_func");
        assert_eq!(symbol.kind, SymbolKind::Function);
        assert_eq!(symbol.branch_name, "main");
        assert_eq!(symbol.last_commit_hash, Some("abc123".to_string()));
    }

    #[test]
    fn test_symbol_with_branch_fields() {
        // 测试分支感知字段
        let symbol = Symbol {
            id: "sym-2".to_string(),
            file_id: "file-1".to_string(),
            name: "another_func".to_string(),
            kind: SymbolKind::Function,
            start_line: 30,
            end_line: 40,
            doc_comment: None,
            code: "fn another_func() {}".to_string(),
            parent_id: None,
            branch_name: "feature/api".to_string(),
            last_commit_hash: None,
        };

        assert_eq!(symbol.branch_name, "feature/api");
        assert!(symbol.last_commit_hash.is_none());
    }

    #[test]
    fn test_symbol_partial_eq() {
        // 测试 PartialEq 考虑新字段
        let symbol1 = Symbol {
            id: "sym-1".to_string(),
            file_id: "file-1".to_string(),
            name: "func".to_string(),
            kind: SymbolKind::Function,
            start_line: 10,
            end_line: 20,
            doc_comment: None,
            code: "fn func() {}".to_string(),
            parent_id: None,
            branch_name: "main".to_string(),
            last_commit_hash: None,
        };

        let mut symbol2 = symbol1.clone();

        // 完全相同的符号应该相等
        assert_eq!(symbol1, symbol2);

        // 修改分支名称应该不相等
        symbol2.branch_name = "feature".to_string();
        assert_ne!(symbol1, symbol2);

        // 修改提交哈希应该不相等
        symbol2.branch_name = symbol1.branch_name.clone();
        symbol2.last_commit_hash = Some("def456".to_string());
        assert_ne!(symbol1, symbol2);
    }
}
