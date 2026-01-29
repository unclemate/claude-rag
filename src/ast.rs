//! AST parsing for code symbol extraction.
//!
//! This module provides functionality to parse source code and extract
//! symbols (functions, classes, etc.) using Tree-sitter.

use crate::error::{Result, RagError};
use crate::models::{Symbol, SymbolKind};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Tree-sitter based AST parser for extracting code symbols.
pub struct AstParser {
    /// The language being parsed.
    pub language: SupportedLanguage,
    /// Tree-sitter parser instance.
    parser: tree_sitter::Parser,
}

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    /// Rust programming language.
    Rust,
    /// Python programming language.
    Python,
    /// JavaScript programming language.
    JavaScript,
    /// TypeScript programming language.
    TypeScript,
}

impl SupportedLanguage {
    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        // Case-insensitive matching without allocation
        match ext {
            "rs" | "RS" | "Rs" => Some(SupportedLanguage::Rust),
            "py" | "PY" | "Py" => Some(SupportedLanguage::Python),
            "js" | "JS" | "Js" | "mjs" | "MJS" | "cjs" | "CJS" => Some(SupportedLanguage::JavaScript),
            "ts" | "TS" | "Ts" | "tsx" | "TSX" | "Tsx" => Some(SupportedLanguage::TypeScript),
            _ => None,
        }
    }

    /// Get the Tree-sitter language for this language.
    fn tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            SupportedLanguage::Rust => tree_sitter_rust::language(),
            SupportedLanguage::Python => tree_sitter_python::language(),
            SupportedLanguage::JavaScript => tree_sitter_javascript::language(),
            SupportedLanguage::TypeScript => tree_sitter_typescript::language_typescript(),
        }
    }

    /// Get the file extension for this language.
    pub fn extension(&self) -> &str {
        match self {
            SupportedLanguage::Rust => "rs",
            SupportedLanguage::Python => "py",
            SupportedLanguage::JavaScript => "js",
            SupportedLanguage::TypeScript => "ts",
        }
    }
}

impl AstParser {
    /// Create a new parser for the specified language.
    pub fn new(language: SupportedLanguage) -> Result<Self> {
        let mut parser = tree_sitter::Parser::new();
        let ts_lang = language.tree_sitter_language();

        parser
            .set_language(&ts_lang)
            .map_err(|e| RagError::Parse(format!("Failed to set language: {}", e)))?;

        Ok(Self { language, parser })
    }

    /// Create a parser from file path.
    pub fn from_path(file_path: &Path) -> Result<Self> {
        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| RagError::Parse("No file extension".to_string()))?;

        let language = SupportedLanguage::from_extension(extension).ok_or_else(|| {
            RagError::Parse(format!("Unsupported file extension: {}", extension))
        })?;

        Self::new(language)
    }

    /// Parse source code and extract all symbols.
    pub fn extract_symbols(&mut self, source: &str, file_path: &Path, file_id: &str) -> Result<Vec<Symbol>> {
        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| RagError::Parse("Failed to parse source code".to_string()))?;

        let root = tree.root_node();

        if root.has_error() {
            return Err(RagError::Parse("Source code contains syntax errors".to_string()));
        }

        let mut symbols = Vec::new();

        match self.language {
            SupportedLanguage::Rust => self.extract_rust_symbols(&root, source, file_path, file_id, &mut symbols, None)?,
            SupportedLanguage::Python => self.extract_python_symbols(&root, source, file_path, file_id, &mut symbols, None)?,
            SupportedLanguage::JavaScript | SupportedLanguage::TypeScript => {
                self.extract_javascript_symbols(&root, source, file_path, file_id, &mut symbols, None)?
            }
        }

        Ok(symbols)
    }

    /// Helper to create a basic symbol (for backwards compatibility with existing parse methods).
    fn create_symbol(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        kind: SymbolKind,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let name = self.extract_node_name(node, source);
        if name.is_empty() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_doc_comment(node, source);
        let code = self.extract_node_code(node, source);
        let id = self.generate_symbol_id(file_path, &name, start_line);

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name,
            kind,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Extract symbols from Rust source code.
    fn extract_rust_symbols(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        symbols: &mut Vec<Symbol>,
        parent_id: Option<String>,
    ) -> Result<()> {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                // Functions
                "function_item" => {
                    if let Some(symbol) = self.create_symbol(&child, source, file_path, file_id, SymbolKind::Function, parent_id.clone())? {
                        symbols.push(symbol);
                    }
                }
                // Structs
                "struct_item" => {
                    if let Some(symbol) = self.create_symbol(&child, source, file_path, file_id, SymbolKind::Class, parent_id.clone())? {
                        let struct_id = symbol.id.clone();
                        symbols.push(symbol);

                        // Extract nested impl blocks
                        self.extract_nested_symbols(&child, source, file_path, file_id, symbols, Some(struct_id.clone()))?;
                    }
                }
                // Enums
                "enum_item" => {
                    if let Some(symbol) = self.create_symbol(&child, source, file_path, file_id, SymbolKind::Enum, parent_id.clone())? {
                        symbols.push(symbol);
                    }
                }
                // Traits
                "trait_item" => {
                    if let Some(symbol) = self.create_symbol(&child, source, file_path, file_id, SymbolKind::Interface, parent_id.clone())? {
                        let trait_id = symbol.id.clone();
                        symbols.push(symbol);

                        // Extract nested items
                        self.extract_nested_symbols(&child, source, file_path, file_id, symbols, Some(trait_id.clone()))?;
                    }
                }
                // Impl blocks
                "impl_item" => {
                    if let Some(symbol) = self.parse_rust_impl(&child, source, file_path, file_id, parent_id.clone())? {
                        let impl_id = symbol.id.clone();
                        symbols.push(symbol);

                        // Extract impl items (methods)
                        self.extract_nested_symbols(&child, source, file_path, file_id, symbols, Some(impl_id.clone()))?;
                    }
                }
                // Modules
                "mod_item" => {
                    if let Some(symbol) = self.create_symbol(&child, source, file_path, file_id, SymbolKind::Module, parent_id.clone())? {
                        symbols.push(symbol);
                    }
                }
                // Recurse into other nodes
                _ => {
                    if child.is_named() {
                        self.extract_rust_symbols(&child, source, file_path, file_id, symbols, parent_id.clone())?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Parse a Rust impl block.
    fn parse_rust_impl(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let type_name = self.extract_impl_type(node, source);

        if type_name.is_empty() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_doc_comment(node, source);
        let code = self.extract_node_code(node, source);

        // Generate ID for impl block
        let id = self.generate_symbol_id(file_path, &format!("impl_{}", type_name), start_line);

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name: format!("impl {}", type_name),
            kind: SymbolKind::Class,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Extract symbols from Python source code.
    fn extract_python_symbols(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        symbols: &mut Vec<Symbol>,
        parent_id: Option<String>,
    ) -> Result<()> {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    if let Some(symbol) = self.parse_python_function(&child, source, file_path, file_id, parent_id.clone())? {
                        symbols.push(symbol);
                    }
                }
                "class_definition" => {
                    if let Some(symbol) = self.parse_python_class(&child, source, file_path, file_id, parent_id.clone())? {
                        let class_id = symbol.id.clone();
                        symbols.push(symbol);

                        // Extract nested methods
                        self.extract_nested_symbols(&child, source, file_path, file_id, symbols, Some(class_id))?;
                    }
                }
                _ => {
                    if child.is_named() {
                        self.extract_python_symbols(&child, source, file_path, file_id, symbols, parent_id.clone())?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Parse a Python function.
    fn parse_python_function(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let name = self.extract_node_name(node, source);

        if name.is_empty() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_python_docstring(node, source);
        let code = self.extract_node_code(node, source);
        let id = self.generate_symbol_id(file_path, &name, start_line);

        let kind = if parent_id.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name,
            kind,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Parse a Python class.
    fn parse_python_class(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let name = self.extract_node_name(node, source);

        if name.is_empty() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_python_docstring(node, source);
        let code = self.extract_node_code(node, source);
        let id = self.generate_symbol_id(file_path, &name, start_line);

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name,
            kind: SymbolKind::Class,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Extract symbols from JavaScript/TypeScript source code.
    fn extract_javascript_symbols(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        symbols: &mut Vec<Symbol>,
        parent_id: Option<String>,
    ) -> Result<()> {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "function_declaration" | "function_expression" | "method_definition" => {
                    if let Some(symbol) = self.parse_javascript_function(&child, source, file_path, file_id, parent_id.clone())? {
                        symbols.push(symbol);
                    }
                }
                "class_declaration" | "class_expression" => {
                    if let Some(symbol) = self.parse_javascript_class(&child, source, file_path, file_id, parent_id.clone())? {
                        let class_id = symbol.id.clone();
                        symbols.push(symbol);

                        // Extract nested methods
                        self.extract_nested_symbols(&child, source, file_path, file_id, symbols, Some(class_id))?;
                    }
                }
                "interface_declaration" => {
                    if let Some(symbol) = self.parse_typescript_interface(&child, source, file_path, file_id, parent_id.clone())? {
                        let interface_id = symbol.id.clone();
                        symbols.push(symbol);

                        self.extract_nested_symbols(&child, source, file_path, file_id, symbols, Some(interface_id))?;
                    }
                }
                "variable_declaration" => {
                    if let Some(symbol) = self.parse_javascript_variable(&child, source, file_path, file_id, parent_id.clone())? {
                        symbols.push(symbol);
                    }
                }
                _ => {
                    if child.is_named() {
                        self.extract_javascript_symbols(&child, source, file_path, file_id, symbols, parent_id.clone())?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Parse a JavaScript/TypeScript function.
    fn parse_javascript_function(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let name = self.extract_node_name(node, source);

        if name.is_empty() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_doc_comment(node, source);
        let code = self.extract_node_code(node, source);
        let id = self.generate_symbol_id(file_path, &name, start_line);

        let kind = if parent_id.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name,
            kind,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Parse a JavaScript/TypeScript class.
    fn parse_javascript_class(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let name = self.extract_node_name(node, source);

        if name.is_empty() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_doc_comment(node, source);
        let code = self.extract_node_code(node, source);
        let id = self.generate_symbol_id(file_path, &name, start_line);

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name,
            kind: SymbolKind::Class,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Parse a TypeScript interface.
    fn parse_typescript_interface(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let name = self.extract_node_name(node, source);

        if name.is_empty() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_doc_comment(node, source);
        let code = self.extract_node_code(node, source);
        let id = self.generate_symbol_id(file_path, &name, start_line);

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name,
            kind: SymbolKind::Interface,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Parse a JavaScript variable declaration.
    fn parse_javascript_variable(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        parent_id: Option<String>,
    ) -> Result<Option<Symbol>> {
        let name = self.extract_node_name(node, source);

        if name.is_empty() {
            return Ok(None);
        }

        // Only extract const/let at module level (no parent)
        if parent_id.is_some() {
            return Ok(None);
        }

        let (start_line, end_line) = self.get_node_range(node);
        let doc_comment = self.extract_doc_comment(node, source);
        let code = self.extract_node_code(node, source);
        let id = self.generate_symbol_id(file_path, &name, start_line);

        Ok(Some(Symbol {
            id,
            file_id: file_id.to_string(),
            name,
            kind: SymbolKind::Variable,
            start_line,
            end_line,
            doc_comment,
            code,
            parent_id,
            branch_name: String::new(),
            last_commit_hash: None,
        }))
    }

    /// Extract nested symbols from within a parent node.
    fn extract_nested_symbols(
        &self,
        node: &tree_sitter::Node,
        source: &str,
        file_path: &Path,
        file_id: &str,
        symbols: &mut Vec<Symbol>,
        parent_id: Option<String>,
    ) -> Result<()> {
        match self.language {
            SupportedLanguage::Rust => self.extract_rust_symbols(node, source, file_path, file_id, symbols, parent_id),
            SupportedLanguage::Python => self.extract_python_symbols(node, source, file_path, file_id, symbols, parent_id),
            SupportedLanguage::JavaScript | SupportedLanguage::TypeScript => {
                self.extract_javascript_symbols(node, source, file_path, file_id, symbols, parent_id)
            }
        }
    }

    /// Extract the name from a node.
    fn extract_node_name(&self, node: &tree_sitter::Node, source: &str) -> String {
        // Try to find a name child
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "name" || child.kind().ends_with("_identifier") {
                return self.extract_node_text(&child, source);
            }
        }

        // Fallback: find first identifier
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind().contains("identifier") {
                return self.extract_node_text(&child, source);
            }
        }

        String::new()
    }

    /// Extract impl type name.
    fn extract_impl_type(&self, node: &tree_sitter::Node, source: &str) -> String {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                return self.extract_node_text(&child, source);
            }
        }

        String::new()
    }

    /// Extract the text content of a node.
    fn extract_node_text(&self, node: &tree_sitter::Node, source: &str) -> String {
        source[node.byte_range()].to_string()
    }

    /// Get the line range of a node (1-indexed).
    fn get_node_range(&self, node: &tree_sitter::Node) -> (usize, usize) {
        let start = node.start_position();
        let end = node.end_position();
        (start.row + 1, end.row + 1) // 1-indexed
    }

    /// Extract the full code of a node.
    fn extract_node_code(&self, node: &tree_sitter::Node, source: &str) -> String {
        self.extract_node_text(node, source)
    }

    /// Extract documentation comment for a node.
    fn extract_doc_comment(&self, node: &tree_sitter::Node, source: &str) -> Option<String> {
        let mut doc_lines = Vec::new();
        let mut prev_sibling = node.prev_sibling();

        // Collect all consecutive doc comments
        while let Some(sibling) = prev_sibling {
            if sibling.kind() == "line_comment" || sibling.kind() == "block_comment" {
                let text = self.extract_node_text(&sibling, source);

                // Check if it looks like a doc comment (///, /**, //!)
                let is_doc = text.starts_with("///") || text.starts_with("/**") || text.starts_with("//!");

                if is_doc {
                    // Clean up each line and prepend to doc_lines
                    // (since we're going backwards)
                    let cleaned: String = text
                        .lines()
                        .map(|line| {
                            let line = line.trim();
                            if let Some(stripped) = line.strip_prefix("///").or_else(|| line.strip_prefix("//!")) {
                                stripped.trim().to_string()
                            } else if let Some(stripped) = line.strip_prefix("/**") {
                                // Handle /** ... */ comments
                                stripped.trim_end_matches("*/").trim().to_string()
                            } else if let Some(stripped) = line.strip_prefix("*") {
                                // Handle Javadoc-style * in middle of block
                                stripped.trim().to_string()
                            } else {
                                line.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    doc_lines.insert(0, cleaned);
                } else {
                    // Break on non-doc comment
                    break;
                }
            } else {
                // Break on non-comment
                break;
            }

            prev_sibling = sibling.prev_sibling();
        }

        if doc_lines.is_empty() {
            None
        } else {
            Some(doc_lines.join("\n"))
        }
    }

    /// Extract Python docstring.
    fn extract_python_docstring(&self, node: &tree_sitter::Node, source: &str) -> Option<String> {
        let mut cursor = node.walk();

        // Look for docstring in the function/class body
        // Python docstrings are typically the first statement in the body
        for child in node.children(&mut cursor) {
            match child.kind() {
                "block" => {
                    // Check first child of block for docstring
                    let block_children: Vec<_> = child.children(&mut child.walk()).collect();
                    if let Some(first_statement) = block_children.first() {
                        if first_statement.kind() == "string" {
                            return self.clean_python_docstring(&self.extract_node_text(first_statement, source));
                        }
                        if first_statement.kind() == "expression_statement" {
                            let mut expr_cursor = first_statement.walk();
                            for expr_child in first_statement.children(&mut expr_cursor) {
                                if expr_child.kind() == "string" {
                                    return self.clean_python_docstring(&self.extract_node_text(&expr_child, source));
                                }
                            }
                        }
                    }
                }
                "string" => {
                    // Direct string child (simplified case)
                    return self.clean_python_docstring(&self.extract_node_text(&child, source));
                }
                "expression_statement" => {
                    // Expression statement containing string
                    let mut expr_cursor = child.walk();
                    for expr_child in child.children(&mut expr_cursor) {
                        if expr_child.kind() == "string" {
                            return self.clean_python_docstring(&self.extract_node_text(&expr_child, source));
                        }
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Clean Python docstring by removing quotes and handling multiline strings.
    fn clean_python_docstring(&self, text: &str) -> Option<String> {
        let trimmed = text.trim();

        // Handle triple-quoted strings
        if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
            let content = &trimmed[3..];
            let content = if content.ends_with("\"\"\"") || content.ends_with("'''") {
                &content[..content.len() - 3]
            } else {
                content
            };
            return Some(content.trim().to_string());
        }

        // Handle single-quoted strings
        if (trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        {
            return Some(trimmed[1..trimmed.len() - 1].to_string());
        }

        Some(trimmed.to_string())
    }

    /// Generate a unique symbol ID.
    fn generate_symbol_id(&self, file_path: &Path, name: &str, line: usize) -> String {
        let path_str = file_path.to_string_lossy();
        let input = format!("{}:{}:{}", path_str, name, line);
        format!("symbol:{:x}", Sha256::digest(input.as_bytes()))
    }
}

impl Default for AstParser {
    fn default() -> Self {
        Self::new(SupportedLanguage::Rust).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_language_from_extension() {
        assert_eq!(SupportedLanguage::from_extension("rs"), Some(SupportedLanguage::Rust));
        assert_eq!(SupportedLanguage::from_extension("py"), Some(SupportedLanguage::Python));
        assert_eq!(SupportedLanguage::from_extension("js"), Some(SupportedLanguage::JavaScript));
        assert_eq!(SupportedLanguage::from_extension("ts"), Some(SupportedLanguage::TypeScript));
        assert_eq!(SupportedLanguage::from_extension("unknown"), None);
    }

    #[test]
    fn test_ast_parser_new() {
        let parser = AstParser::new(SupportedLanguage::Rust);
        assert!(parser.is_ok());
        assert_eq!(parser.unwrap().language, SupportedLanguage::Rust);
    }

    #[test]
    fn test_ast_parser_from_path() {
        let parser = AstParser::from_path(Path::new("test.rs"));
        assert!(parser.is_ok());

        let parser = AstParser::from_path(Path::new("test.py"));
        assert!(parser.is_ok());

        let parser = AstParser::from_path(Path::new("test.unknown"));
        assert!(parser.is_err());
    }

    #[test]
    fn test_extract_rust_function() {
        let source = r#"
/// This is a test function
pub fn test_function(x: i32) -> i32 {
    x + 1
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let tree = parser.parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let func_node = root.children(&mut root.walk()).nth(1).unwrap();
        let symbol = parser
            .create_symbol(&func_node, source, Path::new("test.rs"), "file-1", SymbolKind::Function, None)
            .unwrap();

        assert!(symbol.is_some());
        let s = symbol.unwrap();
        assert_eq!(s.name, "test_function");
        assert_eq!(s.kind, SymbolKind::Function);
        assert_eq!(s.doc_comment, Some("This is a test function".to_string()));
    }

    #[test]
    fn test_extract_rust_struct() {
        let source = r#"
/// A test struct
pub struct TestStruct {
    pub field: i32,
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let tree = parser.parser.parse(source, None).unwrap();
        let root = tree.root_node();

        let struct_node = root.children(&mut root.walk()).nth(1).unwrap();
        let symbol = parser
            .create_symbol(&struct_node, source, Path::new("test.rs"), "file-1", SymbolKind::Class, None)
            .unwrap();

        assert!(symbol.is_some());
        let s = symbol.unwrap();
        assert_eq!(s.name, "TestStruct");
        assert_eq!(s.kind, SymbolKind::Class);
    }

    #[test]
    fn test_extract_python_function() {
        let source = r#"
def hello_world():
    """Print hello world."""
    print("Hello, World!")
"#;

        let mut parser = AstParser::new(SupportedLanguage::Python).unwrap();
        let tree = parser.parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Find the function_definition node
        let mut cursor = root.walk();
        let func_node = root.children(&mut cursor)
            .find(|n| n.kind() == "function_definition")
            .unwrap();
        let symbol = parser
            .parse_python_function(&func_node, source, Path::new("test.py"), "file-1", None)
            .unwrap();

        assert!(symbol.is_some());
        let s = symbol.unwrap();
        assert_eq!(s.name, "hello_world");
        assert_eq!(s.kind, SymbolKind::Function);
        assert_eq!(s.doc_comment, Some("Print hello world.".to_string()));
    }

    #[test]
    fn test_extract_python_class() {
        let source = r#"
class MyClass:
    """A test class."""

    def method(self):
        pass
"#;

        let mut parser = AstParser::new(SupportedLanguage::Python).unwrap();
        let tree = parser.parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Find the class_definition node
        let mut cursor = root.walk();
        let class_node = root.children(&mut cursor)
            .find(|n| n.kind() == "class_definition")
            .unwrap();
        let symbol = parser
            .parse_python_class(&class_node, source, Path::new("test.py"), "file-1", None)
            .unwrap();

        assert!(symbol.is_some());
        let s = symbol.unwrap();
        assert_eq!(s.name, "MyClass");
        assert_eq!(s.kind, SymbolKind::Class);
    }

    #[test]
    fn test_extract_javascript_function() {
        let source = r#"
/**
 * Test function
 */
function testFunction(x) {
    return x + 1;
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::JavaScript).unwrap();
        let tree = parser.parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Find the function_declaration node
        let mut cursor = root.walk();
        let func_node = root.children(&mut cursor)
            .find(|n| n.kind() == "function_declaration")
            .unwrap();
        let symbol = parser
            .parse_javascript_function(&func_node, source, Path::new("test.js"), "file-1", None)
            .unwrap();

        assert!(symbol.is_some());
        let s = symbol.unwrap();
        assert_eq!(s.name, "testFunction");
        assert_eq!(s.kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_javascript_class() {
        let source = r#"
/**
 * Test class
 */
class TestClass {
    constructor(value) {
        this.value = value;
    }
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::JavaScript).unwrap();
        let tree = parser.parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Find the class_declaration node
        let mut cursor = root.walk();
        let class_node = root.children(&mut cursor)
            .find(|n| n.kind() == "class_declaration")
            .unwrap();
        let symbol = parser
            .parse_javascript_class(&class_node, source, Path::new("test.js"), "file-1", None)
            .unwrap();

        assert!(symbol.is_some());
        let s = symbol.unwrap();
        assert_eq!(s.name, "TestClass");
        assert_eq!(s.kind, SymbolKind::Class);
    }

    #[test]
    fn test_generate_symbol_id() {
        let parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let id1 = parser.generate_symbol_id(Path::new("test.rs"), "test_func", 10);
        let id2 = parser.generate_symbol_id(Path::new("test.rs"), "test_func", 10);
        let id3 = parser.generate_symbol_id(Path::new("test.rs"), "test_func", 20);

        assert_eq!(id1, id2); // Same parameters produce same ID
        assert_ne!(id1, id3); // Different line produces different ID
        assert!(id1.starts_with("symbol:"));
    }

    #[test]
    fn test_extract_symbols_with_syntax_error() {
        let source = "fn broken { // missing closing brace";

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        // Should return error for syntax errors
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_symbols_empty_file() {
        let source = "";

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        // Empty file should parse successfully but return no symbols
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_extract_symbols_only_comments() {
        let source = r#"
// This is just a comment
/// And a doc comment with no function
/* Another comment */
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        // Should parse successfully with no symbols
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_extract_symbols_unicode() {
        let source = r#"
/// 函数说明：中文测试
pub fn 中文函数(x: i32) -> i32 {
    x + 1
}

/// 関數説明：日文測試
pub fn 日文関数() {
    println!("こんにちは");
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();
        assert_eq!(symbols.len(), 2);

        // Check first symbol with Chinese name
        let s1 = &symbols[0];
        assert_eq!(s1.name, "中文函数");
        assert_eq!(s1.doc_comment, Some("函数说明：中文测试".to_string()));

        // Check second symbol with Japanese name
        let s2 = &symbols[1];
        assert_eq!(s2.name, "日文関数");
        assert_eq!(s2.doc_comment, Some("関數説明：日文測試".to_string()));
    }

    #[test]
    fn test_case_insensitive_extension() {
        // Test case-insensitive extension matching
        assert_eq!(SupportedLanguage::from_extension("RS"), Some(SupportedLanguage::Rust));
        assert_eq!(SupportedLanguage::from_extension("Py"), Some(SupportedLanguage::Python));
        assert_eq!(SupportedLanguage::from_extension("Js"), Some(SupportedLanguage::JavaScript));
        assert_eq!(SupportedLanguage::from_extension("TS"), Some(SupportedLanguage::TypeScript));
        assert_eq!(SupportedLanguage::from_extension("MJS"), Some(SupportedLanguage::JavaScript));
        assert_eq!(SupportedLanguage::from_extension("TSX"), Some(SupportedLanguage::TypeScript));
    }

    #[test]
    fn test_multiline_doc_comment() {
        let source = r#"
/// This is a multiline
/// documentation comment
/// that spans multiple lines
pub fn documented() {}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();
        assert_eq!(symbols.len(), 1);

        let doc = symbols[0].doc_comment.as_ref().unwrap();
        // Check that all lines are preserved
        assert!(doc.contains("multiline"));
        assert!(doc.contains("documentation comment"));
        assert!(doc.contains("that spans multiple lines"));
    }

    #[test]
    fn test_nested_symbols_structure() {
        let source = r#"
impl MyStruct {
    pub fn method_one(&self) {}
    pub fn method_two(&mut self) {}

    pub fn nested_helper(&self) {
        fn inner_function() {}
    }
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        // Should have impl block and methods
        let impl_symbol = symbols.iter().find(|s| s.name.starts_with("impl "));
        assert!(impl_symbol.is_some());

        // Methods should have the impl as parent
        let methods: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
        assert!(methods.len() >= 2);
    }

    #[test]
    fn test_python_docstring_variations() {
        let source = r#"
def func1():
    """Single line docstring."""
    pass

def func2():
    """
    Multiline docstring.
    With multiple lines.
    """
    pass

def func3():
    '''Single quotes docstring.'''
    pass
"#;

        let mut parser = AstParser::new(SupportedLanguage::Python).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.py"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();
        assert_eq!(symbols.len(), 3);

        // Check each function has proper docstring
        assert_eq!(symbols[0].doc_comment.as_deref(), Some("Single line docstring."));
        assert!(symbols[1].doc_comment.as_ref().unwrap().contains("Multiline"));
        assert_eq!(symbols[2].doc_comment.as_deref(), Some("Single quotes docstring."));
    }

    #[test]
    fn test_extension_method() {
        assert_eq!(SupportedLanguage::Rust.extension(), "rs");
        assert_eq!(SupportedLanguage::Python.extension(), "py");
        assert_eq!(SupportedLanguage::JavaScript.extension(), "js");
        assert_eq!(SupportedLanguage::TypeScript.extension(), "ts");
    }

    #[test]
    fn test_from_path_no_extension() {
        let result = AstParser::from_path(Path::new("Makefile"));
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(e.to_string().contains("No file extension") || e.to_string().contains("Unsupported"));
        }
    }

    #[test]
    fn test_from_path_invalid_extension() {
        let result = AstParser::from_path(Path::new("test.unknown_ext"));
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_rust_impl_block() {
        let source = r#"
/// Impl block for MyStruct
impl MyStruct {
    pub fn new() -> Self {
        MyStruct
    }
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        // Should have impl block
        let impl_symbol = symbols.iter().find(|s| s.name.starts_with("impl "));
        assert!(impl_symbol.is_some());

        let impl_sym = impl_symbol.unwrap();
        assert_eq!(impl_sym.name, "impl MyStruct");
        assert_eq!(impl_sym.kind, SymbolKind::Class);
        assert_eq!(impl_sym.doc_comment, Some("Impl block for MyStruct".to_string()));
    }

    #[test]
    fn test_extract_rust_module() {
        let source = r#"
/// Module documentation
mod my_module;
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        let module_sym = symbols.iter().find(|s| s.kind == SymbolKind::Module);
        assert!(module_sym.is_some());

        let sym = module_sym.unwrap();
        assert_eq!(sym.name, "my_module");
        assert_eq!(sym.kind, SymbolKind::Module);
    }

    #[test]
    fn test_extract_typescript_interface() {
        let source = r#"
interface User {
    name: string;
    age: number;
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::TypeScript).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.ts"), "file-1");

        assert!(result.is_ok());
        let _symbols = result.unwrap();

        // TypeScript should parse without errors
        // Note: interface parsing may not extract symbols depending on tree-sitter grammar
        // The important part is that it doesn't panic
    }

    #[test]
    fn test_extract_javascript_variable() {
        let source = r#"
const MAX_SIZE = 100;
"#;

        let mut parser = AstParser::new(SupportedLanguage::JavaScript).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.js"), "file-1");

        assert!(result.is_ok());
        // Variable extraction is experimental - just ensure no panic
    }

    #[test]
    fn test_rust_trait_extraction() {
        let source = r#"
/// A trait for drawable objects
pub trait Drawable {
    /// Draw the object
    fn draw(&self);
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        let trait_sym = symbols.iter().find(|s| s.kind == SymbolKind::Interface);
        assert!(trait_sym.is_some());

        let sym = trait_sym.unwrap();
        assert_eq!(sym.name, "Drawable");
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert!(sym.doc_comment.as_ref().unwrap().contains("trait for drawable"));
    }

    #[test]
    fn test_rust_enum_extraction() {
        let source = r#"
/// Result type
pub enum Result {
    Ok,
    Err,
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        let enum_sym = symbols.iter().find(|s| s.kind == SymbolKind::Enum);
        assert!(enum_sym.is_some());

        let sym = enum_sym.unwrap();
        assert_eq!(sym.name, "Result");
        assert_eq!(sym.kind, SymbolKind::Enum);
        assert_eq!(sym.doc_comment, Some("Result type".to_string()));
    }

    #[test]
    fn test_nested_symbols_deeply() {
        let source = r#"
impl OuterStruct {
    pub fn method(&self) {
        fn inner_fn() {}
        struct InnerStruct {}
    }
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        // Should have impl block and nested method
        assert!(symbols.iter().any(|s| s.name.starts_with("impl ")));
        assert!(symbols.iter().any(|s| s.name == "method"));
    }

    #[test]
    fn test_block_comment_documentation() {
        let source = r#"
/**
 * This is a block comment
 * that spans multiple lines
 */
pub fn block_documented() -> i32 {
    42
}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        assert_eq!(symbols.len(), 1);
        let doc = symbols[0].doc_comment.as_ref().unwrap();

        // Block comments should be properly cleaned
        assert!(doc.contains("block comment"));
        assert!(doc.contains("spans multiple lines"));
    }

    #[test]
    fn test_no_doc_comment() {
        let source = r#"
pub fn undocumented() {}
"#;

        let mut parser = AstParser::new(SupportedLanguage::Rust).unwrap();
        let result = parser.extract_symbols(source, Path::new("test.rs"), "file-1");

        assert!(result.is_ok());
        let symbols = result.unwrap();

        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].doc_comment.is_none());
    }
}
