//! 代码分块器 - 三级分块策略
//!
//! 本模块实现源码向量化前的分块预处理：
//! - **符号级**: 独立符号（函数/类/接口）作为默认检索单元
//! - **块级**: 大型符号（>500行）的内部切分
//! - **文件级**: 文件摘要（导入/导出/概览）
//!
//! ## 分块策略
//!
//! | 级别 | 阈值 | 用途 |
//! |------|------|------|
//! | 符号级 | 默认 | 核心检索单元，保留完整语义 |
//! | 块级 | >500 行 | 避免超长嵌入，保持上下文 |
//! | 文件级 | 每文件 | 粗粒度查询，文件级概览 |
//!
//! ## 示例
//!
//! ```ignore
//! use crate::code_chunker::{CodeChunker, CodeChunkKind};
//!
//! let chunker = CodeChunker::new();
//!
//! // 符号级分块
//! let chunks = chunker.chunk_symbol(&symbol)?;
//!
//! // 文件级摘要
//! let summary = chunker.create_file_summary(
//!     "src/main.rs",
//!     "main",
//!     &source_code,
//! )?;
//! ```

use crate::ast::AstParser;
use crate::error::Result;
use crate::models::Symbol;
use tracing::warn;

/// 大型符号分块阈值（行数）
///
/// 超过此行数的符号会被切分为多个块，避免单个嵌入过长。
const LARGE_SYMBOL_THRESHOLD: usize = 500;

/// 分块重叠行数
///
/// 切分大型符号时，相邻块之间保留的重叠行数，以维持上下文连贯性。
const CHUNK_OVERLAP_LINES: usize = 10;

/// 每块最大字符数
///
/// Zhipu AI embedding-3 API 的 Token 限制约为 8K。
/// 按保守估计，英文约 4 字符/token，中文约 2 字符/token。
/// 设置 10,000 字符限制确保不超限。
const MAX_CHUNK_CHARS: usize = 10_000;

/// 默认分块行数
///
/// 按平均每行 80 字符计算，100 行约 8,000 字符。
const DEFAULT_CHUNK_LINES: usize = 100;

/// 代码块类型
///
/// 表示不同粒度的代码分块结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeChunkKind {
    /// 独立符号（函数、类、接口等）
    ///
    /// 这是最常见的分块类型，保持符号的完整性。
    Symbol {
        /// 符号数据
        symbol: Symbol,
    },

    /// 大型符号的子块
    ///
    /// 当符号超过 `LARGE_SYMBOL_THRESHOLD` 行时，会被切分为多个块。
    SymbolBlock {
        /// 父符号ID
        symbol_id: String,
        /// 块唯一标识符
        block_id: String,
        /// 起始行号（1-indexed）
        start_line: usize,
        /// 结束行号（1-indexed）
        end_line: usize,
        /// 块的代码内容
        code: String,
    },

    /// 文件级摘要
    ///
    /// 提供文件的导入、导出和概览信息，用于粗粒度查询。
    FileSummary {
        /// Git 分支名称
        branch_name: String,
        /// 导入项列表
        imports: Vec<String>,
        /// 导出项列表
        exports: Vec<String>,
    },
}

impl CodeChunkKind {
    /// 获取块的唯一标识符
    pub fn id(&self) -> String {
        match self {
            CodeChunkKind::Symbol { symbol } => symbol.id.clone(),
            CodeChunkKind::SymbolBlock {
                symbol_id,
                block_id,
                ..
            } => format!("{}:{}", symbol_id, block_id),
            CodeChunkKind::FileSummary {
                branch_name, ..
            } => format!("file_summary:{}", branch_name),
        }
    }

    /// 获取块的文本内容（用于向量化）
    pub fn content(&self) -> String {
        match self {
            CodeChunkKind::Symbol { symbol } => {
                let mut content = String::new();

                // 添加符号签名
                content.push_str(&format!("{}: {}\n", symbol.kind, symbol.name));

                // 添加文档注释（如果有）
                if let Some(doc) = &symbol.doc_comment {
                    content.push_str(&format!("```\n{}\n```\n", doc));
                }

                // 添加代码
                content.push_str(&symbol.code);

                content
            }
            CodeChunkKind::SymbolBlock { code, .. } => code.clone(),
            CodeChunkKind::FileSummary {
                imports,
                exports,
                ..
            } => {
                let mut content = String::new();

                if !imports.is_empty() {
                    content.push_str("Imports:\n");
                    for imp in imports {
                        content.push_str(&format!("  - {}\n", imp));
                    }
                }

                if !exports.is_empty() {
                    content.push_str("\nExports:\n");
                    for exp in exports {
                        content.push_str(&format!("  - {}\n", exp));
                    }
                }

                content
            }
        }
    }
}

/// 代码分块器
///
/// 负责将源代码按照不同粒度进行分块，为后续向量化做准备。
///
/// ## 设计原则
///
/// - **KISS**: 三种分块类型，职责清晰
/// - **DRY**: 复用 AstParser 进行符号提取
/// - **YAGNI**: 仅实现当前需要的分块策略
pub struct CodeChunker;

impl CodeChunker {
    /// 创建新的代码分块器
    pub fn new() -> Self {
        Self
    }

    /// 对符号进行分块
    ///
    /// 根据符号大小决定分块策略：
    /// - 小型符号（≤500行 且 完整内容≤MAX_CHUNK_CHARS字符）：保持完整
    /// - 大型符号（>500行 或 完整内容>MAX_CHUNK_CHARS字符）：切分为多个块
    ///
    /// # 参数
    ///
    /// * `symbol` - 要分块的符号
    ///
    /// # 返回
    ///
    /// 分块结果向量
    pub fn chunk_symbol(&self, symbol: &Symbol) -> Result<Vec<CodeChunkKind>> {
        let line_count = symbol.end_line - symbol.start_line + 1;

        // 计算完整内容的字符数（包含签名、文档、代码）
        let full_content = self.format_symbol_content(symbol);
        let char_count = full_content.len();

        // 检查是否需要分块（行数过多 或 字符数过多）
        let needs_chunking = line_count > LARGE_SYMBOL_THRESHOLD || char_count > MAX_CHUNK_CHARS;

        if needs_chunking {
            self.chunk_large_symbol(symbol)
        } else {
            Ok(vec![CodeChunkKind::Symbol {
                symbol: symbol.clone(),
            }])
        }
    }

    /// 格式化符号的完整内容（用于计算字符数）
    fn format_symbol_content(&self, symbol: &Symbol) -> String {
        let mut content = String::new();

        // 添加符号签名
        content.push_str(&format!("{}: {}\n", symbol.kind, symbol.name));

        // 添加文档注释（如果有）
        if let Some(doc) = &symbol.doc_comment {
            content.push_str(&format!("```\n{}\n```\n", doc));
        }

        // 添加代码
        content.push_str(&symbol.code);

        content
    }

    /// 切分大型符号为多个块
    ///
    /// 按固定行数切分，相邻块之间保留重叠行以维持上下文。
    /// 同时检查字符数，超过限制的块将被跳过。
    ///
    /// # 切分策略
    ///
    /// 1. 首先包含符号签名和文档注释
    /// 2. 按 `DEFAULT_CHUNK_LINES` 行为单位切分函数体
    /// 3. 检查字符数，超过 `MAX_CHUNK_CHARS` 的块跳过
    /// 4. 相邻块之间保留 `CHUNK_OVERLAP_LINES` 行重叠
    fn chunk_large_symbol(&self, symbol: &Symbol) -> Result<Vec<CodeChunkKind>> {
        let lines: Vec<&str> = symbol.code.lines().collect();
        let total_lines = lines.len();

        // 找到函数体开始位置（跳过签名和空行）
        let body_start = lines
            .iter()
            .position(|l| l.trim().starts_with('{'))
            .unwrap_or(0);

        let mut chunks = Vec::new();
        let mut block_index = 0;
        let mut skipped_count = 0;

        // 从函数体开始切分
        let mut current_line = body_start;

        // 提取签名和文档（用于第一个块）
        let header_lines: Vec<&str> = symbol.code.lines().take(body_start).collect();
        let header_code = if header_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", header_lines.join("\n"))
        };

        while current_line < total_lines {
            let end_line = (current_line + DEFAULT_CHUNK_LINES).min(total_lines);

            // 构建块的代码
            let mut block_code = String::new();

            // 添加签名和文档（仅第一个块）
            if block_index == 0 {
                block_code.push_str(&header_code);
            }

            // 添加当前块的代码行
            let block_lines: String = lines[current_line..end_line].join("\n");
            block_code.push_str(&block_lines);

            // 如果不是最后一块，添加省略标记
            if end_line < total_lines {
                block_code.push_str("\n    // ... [truncated]");
            }

            // 检查字符数，超过限制则跳过此块
            if block_code.len() > MAX_CHUNK_CHARS {
                warn!(
                    "Skipping chunk {} of symbol {}: too large ({} chars > {} limit)",
                    block_index,
                    symbol.name,
                    block_code.len(),
                    MAX_CHUNK_CHARS
                );
                skipped_count += 1;
                current_line = end_line;
                block_index += 1;
                continue;
            }

            chunks.push(CodeChunkKind::SymbolBlock {
                symbol_id: symbol.id.clone(),
                block_id: format!("block_{}", block_index),
                start_line: symbol.start_line + current_line,
                end_line: symbol.start_line + end_line,
                code: block_code,
            });

            // 移动到下一个块，保留重叠
            // 只有当还有更多内容时才移动，否则退出循环
            if end_line >= total_lines {
                break;
            }
            current_line = end_line.saturating_sub(CHUNK_OVERLAP_LINES);
            block_index += 1;
        }

        if skipped_count > 0 {
            warn!(
                "Skipped {} chunks from symbol {} due to size limit",
                skipped_count, symbol.name
            );
        }

        Ok(chunks)
    }

    /// 创建文件级摘要
    ///
    /// 提取文件的导入和导出信息，用于粗粒度查询。
    ///
    /// # 参数
    ///
    /// * `_file_path` - 文件路径
    /// * `branch_name` - Git 分支名称
    /// * `source` - 源代码内容
    /// * `language` - 编程语言
    ///
    /// # 返回
    ///
    /// 文件摘要块
    pub fn create_file_summary(
        &self,
        _file_path: &str,
        branch_name: &str,
        source: &str,
        language: &str,
    ) -> Result<CodeChunkKind> {
        let (imports, exports) = self.extract_imports_exports(source, language)?;

        Ok(CodeChunkKind::FileSummary {
            branch_name: branch_name.to_string(),
            imports,
            exports,
        })
    }

    /// 提取导入和导出
    ///
    /// 根据编程语言提取相应的导入/导出语句。
    fn extract_imports_exports(
        &self,
        source: &str,
        language: &str,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let mut imports = Vec::new();
        let mut exports = Vec::new();

        match language {
            "rs" | "rust" => {
                self.extract_rust_imports_exports(source, &mut imports, &mut exports);
            }
            "py" | "python" => {
                self.extract_python_imports_exports(source, &mut imports, &mut exports);
            }
            "js" | "javascript" | "ts" | "typescript" => {
                self.extract_js_imports_exports(source, &mut imports, &mut exports);
            }
            _ => {
                // 未知语言，返回空列表
            }
        }

        Ok((imports, exports))
    }

    /// 提取 Rust 的导入和导出
    fn extract_rust_imports_exports(
        &self,
        source: &str,
        imports: &mut Vec<String>,
        exports: &mut Vec<String>,
    ) {
        for line in source.lines() {
            let trimmed = line.trim();

            // use 语句
            if trimmed.starts_with("use ") {
                let import = trimmed
                    .strip_prefix("use ")
                    .unwrap()
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                imports.push(import);
            }

            // pub 导出（仅模块级）
            if trimmed.starts_with("pub ") && trimmed.contains("fn ")
                || trimmed.starts_with("pub ") && trimmed.contains("struct ")
                || trimmed.starts_with("pub ") && trimmed.contains("enum ")
                || trimmed.starts_with("pub ") && trimmed.contains("trait ")
            {
                exports.push(trimmed.to_string());
            }
        }
    }

    /// 提取 Python 的导入和导出
    fn extract_python_imports_exports(
        &self,
        source: &str,
        imports: &mut Vec<String>,
        exports: &mut Vec<String>,
    ) {
        for line in source.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                imports.push(trimmed.to_string());
            }
        }

        // __all__ 导出列表
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("__all__") {
                if let Some(content) = trimmed.strip_prefix("__all__") {
                    exports.push(content.trim().to_string());
                }
            }
        }
    }

    /// 提取 JavaScript/TypeScript 的导入和导出
    fn extract_js_imports_exports(
        &self,
        source: &str,
        imports: &mut Vec<String>,
        exports: &mut Vec<String>,
    ) {
        for line in source.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("import ") {
                imports.push(trimmed.to_string());
            }

            if trimmed.starts_with("export ") {
                exports.push(trimmed.to_string());
            }
        }
    }

    /// 批量分块多个符号
    ///
    /// # 参数
    ///
    /// * `symbols` - 符号集合
    ///
    /// # 返回
    ///
    /// 所有分块的扁平化列表
    pub fn chunk_symbols(&self, symbols: &[Symbol]) -> Result<Vec<CodeChunkKind>> {
        let mut all_chunks = Vec::new();

        for symbol in symbols {
            let chunks = self.chunk_symbol(symbol)?;
            all_chunks.extend(chunks);
        }

        Ok(all_chunks)
    }

    /// 从源代码提取并分块所有符号
    ///
    /// 结合 AstParser 和分块逻辑的便捷方法。
    ///
    /// # 参数
    ///
    /// * `source` - 源代码
    /// * `file_path` - 文件路径
    /// * `file_id` - 文件ID
    /// * `branch_name` - Git 分支名称
    ///
    /// # 返回
    ///
    /// 所有代码块（包含符号和文件摘要）
    pub fn extract_and_chunk(
        &self,
        source: &str,
        file_path: &std::path::Path,
        file_id: &str,
        branch_name: &str,
    ) -> Result<(Vec<CodeChunkKind>, Vec<Symbol>)> {
        // 创建解析器
        let mut parser = AstParser::from_path(file_path)?;

        // 提取符号
        let symbols = parser.extract_symbols(source, file_path, file_id)?;

        // 为符号添加分支信息
        // 注意：这里需要在 Symbol 模型添加 branch_name 字段后实现
        // 目前暂时忽略
        let _ = branch_name;

        // 分块符号
        let chunks = self.chunk_symbols(&symbols)?;

        // 创建文件摘要
        let language = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown");

        let summary = self.create_file_summary(
            &file_path.display().to_string(),
            branch_name,
            source,
            language,
        )?;

        let mut all_chunks = chunks;
        all_chunks.push(summary);

        Ok((all_chunks, symbols))
    }
}

impl Default for CodeChunker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SymbolKind;
    use std::path::Path;

    #[test]
    fn test_code_chunker_new() {
        let _chunker = CodeChunker::new();
        // 成功创建即可
    }

    #[test]
    fn test_chunk_small_symbol() {
        let chunker = CodeChunker::new();

        let symbol = Symbol {
            id: "test_symbol".to_string(),
            file_id: "file_1".to_string(),
            name: "test_function".to_string(),
            kind: SymbolKind::Function,
            start_line: 10,
            end_line: 20, // 只有 10 行
            doc_comment: Some("Test function".to_string()),
            code: "fn test_function() {\n    println!(\"test\");\n}".to_string(),
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        let chunks = chunker.chunk_symbol(&symbol).unwrap();

        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0], CodeChunkKind::Symbol { .. }));

        if let CodeChunkKind::Symbol { symbol: s } = &chunks[0] {
            assert_eq!(s.name, "test_function");
        }
    }

    #[test]
    fn test_chunk_large_symbol() {
        let chunker = CodeChunker::new();

        // 创建一个超过阈值的符号
        let mut code = String::from("fn large_function() {\n");
        for i in 0..600 {
            code.push_str(&format!("    println!(\"line {}\");\n", i));
        }
        code.push('}');

        let symbol = Symbol {
            id: "large_symbol".to_string(),
            file_id: "file_1".to_string(),
            name: "large_function".to_string(),
            kind: SymbolKind::Function,
            start_line: 1,
            end_line: 602,
            doc_comment: None,
            code,
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        let chunks = chunker.chunk_symbol(&symbol).unwrap();

        // 应该被切分为多个块
        assert!(chunks.len() > 1);

        // 验证块类型
        for chunk in &chunks {
            assert!(matches!(chunk, CodeChunkKind::SymbolBlock { .. }));
        }
    }

    #[test]
    fn test_create_file_summary_rust() {
        let chunker = CodeChunker::new();

        let source = r#"
use std::collections::HashMap;
use crate::models::Symbol;

pub fn public_function() {}
fn private_function() {}
pub struct PublicStruct {}
struct PrivateStruct {}
"#;

        let summary = chunker
            .create_file_summary("src/test.rs", "main", source, "rust")
            .unwrap();

        if let CodeChunkKind::FileSummary { imports, exports, .. } = summary {
            assert_eq!(imports.len(), 2);
            assert!(imports.contains(&"std::collections::HashMap".to_string()));
            assert!(imports.contains(&"crate::models::Symbol".to_string()));

            assert!(exports.len() >= 2);
            assert!(exports.iter().any(|e| e.contains("public_function")));
            assert!(exports.iter().any(|e| e.contains("PublicStruct")));
        } else {
            panic!("Expected FileSummary");
        }
    }

    #[test]
    fn test_create_file_summary_python() {
        let chunker = CodeChunker::new();

        let source = r#"
import os
from typing import List

__all__ = ['public_func', 'PublicClass']

def public_func():
    pass

class PublicClass:
    pass
"#;

        let summary = chunker
            .create_file_summary("test.py", "main", source, "python")
            .unwrap();

        if let CodeChunkKind::FileSummary { imports, exports, .. } = summary {
            assert_eq!(imports.len(), 2);
            assert!(exports.len() > 0);
        } else {
            panic!("Expected FileSummary");
        }
    }

    #[test]
    fn test_create_file_summary_javascript() {
        let chunker = CodeChunker::new();

        let source = r#"
import React from 'react';
import { useState } from 'hooks';

export function Component() {}
export const value = 42;
"#;

        let summary = chunker
            .create_file_summary("Component.jsx", "main", source, "javascript")
            .unwrap();

        if let CodeChunkKind::FileSummary { imports, exports, .. } = summary {
            assert_eq!(imports.len(), 2);
            assert_eq!(exports.len(), 2);
        } else {
            panic!("Expected FileSummary");
        }
    }

    #[test]
    fn test_chunk_symbols_batch() {
        let chunker = CodeChunker::new();

        let symbols = vec![
            Symbol {
                id: "sym1".to_string(),
                file_id: "file_1".to_string(),
                name: "func1".to_string(),
                kind: SymbolKind::Function,
                start_line: 1,
                end_line: 10,
                doc_comment: None,
                code: "fn func1() {}".to_string(),
                parent_id: None,
                branch_name: String::new(),
                last_commit_hash: None,
            },
            Symbol {
                id: "sym2".to_string(),
                file_id: "file_1".to_string(),
                name: "func2".to_string(),
                kind: SymbolKind::Function,
                start_line: 12,
                end_line: 22,
                doc_comment: None,
                code: "fn func2() {}".to_string(),
                parent_id: None,
                branch_name: String::new(),
                last_commit_hash: None,
            },
        ];

        let chunks = chunker.chunk_symbols(&symbols).unwrap();

        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn test_code_chunk_kind_id() {
        let symbol = Symbol {
            id: "test_id".to_string(),
            file_id: "file_1".to_string(),
            name: "test".to_string(),
            kind: SymbolKind::Function,
            start_line: 1,
            end_line: 10,
            doc_comment: None,
            code: "fn test() {}".to_string(),
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        let chunk = CodeChunkKind::Symbol {
            symbol: symbol.clone(),
        };

        assert_eq!(chunk.id(), "test_id");

        let block = CodeChunkKind::SymbolBlock {
            symbol_id: "sym_1".to_string(),
            block_id: "block_0".to_string(),
            start_line: 1,
            end_line: 100,
            code: "code".to_string(),
        };

        assert_eq!(block.id(), "sym_1:block_0");
    }

    #[test]
    fn test_code_chunk_kind_content() {
        let symbol = Symbol {
            id: "test_id".to_string(),
            file_id: "file_1".to_string(),
            name: "test_func".to_string(),
            kind: SymbolKind::Function,
            start_line: 1,
            end_line: 10,
            doc_comment: Some("Test documentation".to_string()),
            code: "fn test_func() {\n    println!(\"test\");\n}".to_string(),
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        let chunk = CodeChunkKind::Symbol {
            symbol: symbol.clone(),
        };

        let content = chunk.content();

        assert!(content.contains("Function: test_func"));
        assert!(content.contains("Test documentation"));
        assert!(content.contains("fn test_func()"));
    }

    #[test]
    fn test_large_symbol_chunk_preserves_signature() {
        let chunker = CodeChunker::new();

        let mut code = String::from("fn large_func(x: i32) -> i32 {\n");
        for i in 0..600 {
            code.push_str(&format!("    let _ = {};\n", i));
        }
        code.push('}');

        let symbol = Symbol {
            id: "large".to_string(),
            file_id: "file_1".to_string(),
            name: "large_func".to_string(),
            kind: SymbolKind::Function,
            start_line: 1,
            end_line: 602,
            doc_comment: None,
            code,
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        let chunks = chunker.chunk_symbol(&symbol).unwrap();

        // 第一个块应该包含函数签名
        if let CodeChunkKind::SymbolBlock { code, .. } = &chunks[0] {
            assert!(code.contains("fn large_func"));
        } else {
            panic!("First chunk should be SymbolBlock");
        }
    }

    #[test]
    fn test_extract_and_chunk() {
        let chunker = CodeChunker::new();

        let source = r#"
/// A test function
pub fn test_func() -> i32 {
    42
}

/// A test struct
pub struct TestStruct {
    value: i32,
}
"#;

        let result = chunker.extract_and_chunk(
            source,
            Path::new("test.rs"),
            "file_1",
            "main",
        );

        assert!(result.is_ok());
        let (chunks, symbols) = result.unwrap();

        // 应该有符号和文件摘要
        assert!(chunks.len() > 2);
        assert!(symbols.len() >= 2);

        // 验证有文件摘要
        assert!(chunks.iter().any(|c| matches!(c, CodeChunkKind::FileSummary { .. })));
    }

    #[test]
    fn test_constants() {
        // 验证常量定义
        assert!(LARGE_SYMBOL_THRESHOLD == 500);
        assert!(CHUNK_OVERLAP_LINES == 10);
    }
}
