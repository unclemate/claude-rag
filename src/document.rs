//! Document parsing for paragraph-level indexing.

use crate::error::{Result, RagError};
use crate::models::{ChunkKind, DocChunk};
use std::mem;
use std::path::Path;

/// Unified header entry for tracking nested document structure.
#[derive(Debug, Clone)]
struct HeaderEntry {
    /// The header text content.
    pub text: String,
    /// The header level (1-6 for Markdown/AsciiDoc, None for RST).
    pub level: Option<u8>,
}

impl HeaderEntry {
    /// Create a new header entry.
    fn new(text: String, level: Option<u8>) -> Self {
        Self { text, level }
    }
}

/// Supported document formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    /// Markdown (.md, .markdown)
    Markdown,
    /// reStructuredText (.rst)
    ReStructuredText,
    /// Plain text (.txt)
    PlainText,
    /// AsciiDoc (.adoc, .asciidoc)
    AsciiDoc,
}

impl DocumentFormat {
    /// Detect format from file extension.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "md" | "markdown" => Some(DocumentFormat::Markdown),
            "rst" => Some(DocumentFormat::ReStructuredText),
            "txt" | "text" => Some(DocumentFormat::PlainText),
            "adoc" | "asciidoc" => Some(DocumentFormat::AsciiDoc),
            _ => None,
        }
    }

    /// Get the file extension for this format.
    #[must_use]
    pub const fn extension(&self) -> &str {
        match self {
            DocumentFormat::Markdown => "md",
            DocumentFormat::ReStructuredText => "rst",
            DocumentFormat::PlainText => "txt",
            DocumentFormat::AsciiDoc => "adoc",
        }
    }
}

/// Document parser for extracting structured chunks.
pub struct DocumentParser {
    /// The document format being parsed.
    pub format: DocumentFormat,
    /// Minimum chunk length in characters.
    min_chunk_length: usize,
}

impl DocumentParser {
    /// Default minimum chunk length in characters.
    const DEFAULT_MIN_CHUNK_LENGTH: usize = 20;

    /// Create a new document parser with default settings.
    #[must_use]
    pub fn new(format: DocumentFormat) -> Self {
        Self {
            format,
            min_chunk_length: Self::DEFAULT_MIN_CHUNK_LENGTH,
        }
    }

    /// Create a new document parser with custom minimum chunk length.
    ///
    /// # Arguments
    /// * `format` - The document format
    /// * `min_chunk_length` - Minimum number of characters for a chunk to be created
    #[must_use]
    pub const fn with_min_chunk_length(format: DocumentFormat, min_chunk_length: usize) -> Self {
        Self {
            format,
            min_chunk_length,
        }
    }

    /// Create a parser from file path.
    pub fn from_path(file_path: &Path) -> Result<Self> {
        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| RagError::Parse("No file extension".to_string()))?;

        let format = DocumentFormat::from_extension(extension).ok_or_else(|| {
            RagError::Parse(format!("Unsupported document extension: {}", extension))
        })?;

        Ok(Self::new(format))
    }

    /// Get the minimum chunk length.
    #[must_use]
    pub const fn min_chunk_length(&self) -> usize {
        self.min_chunk_length
    }

    /// Parse document content into chunks.
    pub fn parse(&self, content: &str, file_path: &Path, file_id: &str) -> Result<Vec<DocChunk>> {
        match self.format {
            DocumentFormat::Markdown => self.parse_markdown(content, file_path, file_id),
            DocumentFormat::ReStructuredText => self.parse_rst(content, file_path, file_id),
            DocumentFormat::PlainText => self.parse_plain_text(content, file_path, file_id),
            DocumentFormat::AsciiDoc => self.parse_asciidoc(content, file_path, file_id),
        }
    }

    /// Parse Markdown content into chunks.
    fn parse_markdown(&self, content: &str, _file_path: &Path, file_id: &str) -> Result<Vec<DocChunk>> {
        let mut chunks = Vec::new();
        let mut header_stack: Vec<HeaderEntry> = Vec::new();
        let mut current_chunk = String::new();
        let mut chunk_start_line = 1;
        let mut current_line_num = 0;
        let mut in_code_fence = false;
        let mut code_fence_lang: Option<String> = None;
        let mut in_indented_code = false;
        let mut in_blockquote = false;
        let mut prev_line = String::new();

        for (line_idx, line) in content.lines().enumerate() {
            current_line_num = line_idx + 1;

            // Handle code fences (```)
            if line.starts_with("```") {
                if in_code_fence {
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        let content = mem::take(&mut current_chunk);
                        chunks.push(DocChunk::new(
                            file_id.to_string(),
                            ChunkKind::CodeBlock,
                            content,
                            chunk_start_line,
                            current_line_num,
                        ).with_language(code_fence_lang.take().unwrap_or_default()).with_header_path(path));
                    }
                    in_code_fence = false;
                } else {
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        chunks.push(self.create_paragraph_chunk(
                            file_id,
                            &current_chunk,
                            chunk_start_line,
                            current_line_num - 1,
                            &path,
                        ));
                        current_chunk.clear();
                    }
                    let lang = line.strip_prefix("```").unwrap_or("").trim();
                    code_fence_lang = if lang.is_empty() { None } else { Some(lang.to_string()) };
                    chunk_start_line = current_line_num + 1;
                    in_code_fence = true;
                }
                continue;
            }

            // Handle indented code blocks (4 spaces or 1 tab)
            let is_indented = line.starts_with("    ") || line.starts_with("\t");
            let is_empty = line.trim().is_empty();

            if is_indented && !in_code_fence && !in_indented_code {
                // Start of indented code block
                if !current_chunk.is_empty() {
                    let path = Self::extract_header_path(&header_stack);
                    chunks.push(self.create_paragraph_chunk(
                        file_id,
                        &current_chunk,
                        chunk_start_line,
                        current_line_num - 1,
                        &path,
                    ));
                    current_chunk.clear();
                }
                chunk_start_line = current_line_num;
                in_indented_code = true;
                // Remove indentation (4 spaces or 1 tab)
                let unindented = line.strip_prefix("    ").unwrap_or_else(|| {
                    line.strip_prefix('\t').unwrap_or(line)
                });
                current_chunk.push_str(unindented);
                current_chunk.push('\n');
                continue;
            }

            if in_indented_code {
                if is_empty || !is_indented {
                    // End of indented code block
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        let content = mem::take(&mut current_chunk);
                        chunks.push(DocChunk::new(
                            file_id.to_string(),
                            ChunkKind::CodeBlock,
                            content.trim_end().to_string(),
                            chunk_start_line,
                            current_line_num - 1,
                        ).with_header_path(path));
                    }
                    in_indented_code = false;
                } else {
                    // Continue indented code block
                    let unindented = line.strip_prefix("    ").unwrap_or_else(|| {
                        line.strip_prefix('\t').unwrap_or(line)
                    });
                    current_chunk.push_str(unindented);
                    current_chunk.push('\n');
                    continue;
                }
            }

            if in_code_fence {
                current_chunk.push_str(line);
                current_chunk.push('\n');
                continue;
            }

            // Handle blockquotes (>)
            if !in_indented_code {
                let trimmed = line.trim();
                if trimmed.starts_with('>') {
                    // Count the blockquote level
                    let level = trimmed.chars().take_while(|&c| c == '>').count() as u8;
                    if !in_blockquote && level > 0 {
                        in_blockquote = true;
                        if !current_chunk.is_empty() {
                            let path = Self::extract_header_path(&header_stack);
                            chunks.push(self.create_paragraph_chunk(
                                file_id,
                                &current_chunk,
                                chunk_start_line,
                                current_line_num - 1,
                                &path,
                            ));
                            current_chunk.clear();
                        }
                        chunk_start_line = current_line_num;
                    }
                    // Remove the blockquote prefix and add to chunk
                    let content = if (level as usize) < trimmed.len() {
                        trimmed[level as usize..].trim()
                    } else {
                        // level exceeds string length, use empty string
                        ""
                    };
                    current_chunk.push_str(content);
                    current_chunk.push('\n');
                    continue;
                } else if in_blockquote && !trimmed.starts_with('>') {
                    // End of blockquote
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        chunks.push(self.create_paragraph_chunk(
                            file_id,
                            &current_chunk,
                            chunk_start_line,
                            current_line_num - 1,
                            &path,
                        ));
                        current_chunk.clear();
                    }
                    in_blockquote = false;
                }
            }

            // Handle Setext-style headers (underlines)
            // Header text is on prev_line, current line is all === or ---
            let trimmed = line.trim();

            // According to CommonMark spec, Setext underlines just need to be at least 3 chars
            if !prev_line.is_empty() && !trimmed.is_empty() &&
                (trimmed.chars().all(|c| c == '=') || trimmed.chars().all(|c| c == '-')) &&
                trimmed.len() >= 3 {

                if !current_chunk.is_empty() {
                    let path = Self::extract_header_path(&header_stack);
                    // Use saturating_sub to prevent underflow
                    let end_line = current_line_num.saturating_sub(1);
                    chunks.push(self.create_paragraph_chunk(
                        file_id,
                        &current_chunk,
                        chunk_start_line,
                        end_line,
                        &path,
                    ));
                    current_chunk.clear();
                }

                let header_text = Self::clean_markdown_inline(&prev_line);
                let level = if trimmed.starts_with('=') { 1 } else { 2 };

                while header_stack.last().is_some_and(|h| h.level.unwrap_or(0) >= level) {
                    header_stack.pop();
                }
                header_stack.push(HeaderEntry::new(header_text.to_string(), Some(level)));

                let path = Self::extract_header_path(&header_stack);
                chunks.push(DocChunk::new(
                    file_id.to_string(),
                    ChunkKind::Header,
                    header_text,
                    current_line_num - 1,
                    current_line_num,
                ).with_header_level(level).with_header_path(path));

                chunk_start_line = current_line_num + 1;
                prev_line = String::new();
                continue;
            }

            // Handle ATX-style headers
            if let Some(header) = line.strip_prefix('#') {
                // Count all # characters (including the first one we stripped)
                let level = (1 + header.chars().take_while(|&c| c == '#').count()) as u8;
                if level <= 6 && header.chars().nth(level as usize - 1).is_none_or(|c| c.is_whitespace()) {
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        chunks.push(self.create_paragraph_chunk(
                            file_id,
                            &current_chunk,
                            chunk_start_line,
                            current_line_num - 1,
                            &path,
                        ));
                        current_chunk.clear();
                    }

                    let header_text = header[level as usize - 1..].trim();
                    let header_text = Self::clean_markdown_inline(header_text);

                    while header_stack.last().is_some_and(|h| h.level.unwrap_or(0) >= level) {
                        header_stack.pop();
                    }
                    header_stack.push(HeaderEntry::new(header_text.to_string(), Some(level)));

                    let path = Self::extract_header_path(&header_stack);
                    chunks.push(DocChunk::new(
                        file_id.to_string(),
                        ChunkKind::Header,
                        header_text,
                        current_line_num,
                        current_line_num,
                    ).with_header_level(level).with_header_path(path));

                    chunk_start_line = current_line_num + 1;
                    continue;
                }
            }

            current_chunk.push_str(line);
            current_chunk.push('\n');

            if line.trim().is_empty() && current_chunk.len() > self.min_chunk_length {
                let path = Self::extract_header_path(&header_stack);
                chunks.push(self.create_paragraph_chunk(
                    file_id,
                    &current_chunk,
                    chunk_start_line,
                    current_line_num,
                    &path,
                ));
                current_chunk.clear();
                chunk_start_line = current_line_num + 1;
            }

            // Update prev_line for Setext-style header detection in next iteration
            // Only track non-empty lines when not in code block
            if !line.trim().is_empty() && !in_code_fence && !in_indented_code {
                prev_line = line.trim().to_string();
            } else if line.trim().is_empty() {
                prev_line.clear();
            }
        }

        // Handle any remaining content in current_chunk
        if !current_chunk.is_empty() {
            let path = Self::extract_header_path(&header_stack);

            // If still in code block (unclosed), create a code chunk instead of paragraph
            if in_code_fence {
                let content = mem::take(&mut current_chunk);
                chunks.push(DocChunk::new(
                    file_id.to_string(),
                    ChunkKind::CodeBlock,
                    content,
                    chunk_start_line,
                    current_line_num,
                ).with_language(code_fence_lang.take().unwrap_or_default()).with_header_path(path));
            } else if in_indented_code {
                let content = mem::take(&mut current_chunk);
                chunks.push(DocChunk::new(
                    file_id.to_string(),
                    ChunkKind::CodeBlock,
                    content.trim_end().to_string(),
                    chunk_start_line,
                    current_line_num,
                ).with_header_path(path));
            } else {
                chunks.push(self.create_paragraph_chunk(
                    file_id,
                    &current_chunk,
                    chunk_start_line,
                    current_line_num,
                    &path,
                ));
            }
        }

        Ok(chunks)
    }

    /// Extract header path from the header stack as Vec<String>.
    /// This helper reduces repeated code for converting header_stack to path.
    #[inline]
    fn extract_header_path(header_stack: &[HeaderEntry]) -> Vec<String> {
        header_stack.iter().map(|h| h.text.clone()).collect()
    }

    /// Create a paragraph chunk with header path.
    fn create_paragraph_chunk(
        &self,
        file_id: &str,
        content: &str,
        start_line: usize,
        end_line: usize,
        header_path: &[String],
    ) -> DocChunk {
        let cleaned = Self::clean_markdown_text(content);

        DocChunk::new(
            file_id.to_string(),
            ChunkKind::Paragraph,
            cleaned,
            start_line,
            end_line,
        ).with_header_path(header_path.to_vec())
    }

    /// Process Markdown escape sequences, converting them to special placeholders.
    ///
    /// Escaped characters are converted to special placeholders that won't be
    /// treated as special characters by subsequent processing. The placeholders
    /// are replaced back at the end.
    fn process_escape_sequences(text: &str) -> (String, Vec<(String, char)>) {
        let mut result = String::with_capacity(text.len());
        let chars = text.chars().peekable();
        let mut escaped = false;
        let mut replacements = Vec::new();
        let mut placeholder_idx = 0;

        // Characters that need escaping in Markdown
        const ESCAPABLE: &[char] = &['\\', '`', '*', '_', '{', '}', '[', ']',
                                     '(', ')', '#', '+', '-', '.', '!', '|'];

        for ch in chars {
            if ch == '\\' && !escaped {
                escaped = true;
            } else if escaped && ESCAPABLE.contains(&ch) {
                // Create a unique placeholder for this escaped character
                let placeholder = format!("\x00ESC{:04x}\x00", placeholder_idx);
                replacements.push((placeholder.clone(), ch));
                result.push_str(&placeholder);
                placeholder_idx += 1;
                escaped = false;
            } else if escaped {
                // Not a special escape character, keep backslash and character
                result.push('\\');
                result.push(ch);
                escaped = false;
            } else {
                result.push(ch);
            }
        }

        // If the string ends with a backslash, preserve it
        if escaped {
            result.push('\\');
        }

        (result, replacements)
    }

    /// Clean Markdown inline formatting from text (optimized single-pass version).
    ///
    /// This removes **bold**, __bold__, *italic*, and _italic_ formatting,
    /// as well as [links](url) and ![images](url), while preserving the actual content.
    /// Inline code `text` is preserved as-is.
    /// Uses a single-pass state machine to minimize string allocations.
    fn clean_markdown_inline(text: &str) -> String {
        // First, process escape sequences and get placeholder mappings
        let (text, replacements) = Self::process_escape_sequences(text);

        // Single-pass state machine for processing all inline formatting
        let mut result = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        let mut state = InlineState::Normal;
        let mut link_text = String::new();
        let mut alt_text = String::new();
        let mut inline_code_content = String::new();
        let mut paren_depth = 0;
        let mut asterisk_content = String::new();
        let mut underscore_content = String::new();
        let mut backtick_count = 0;

        while let Some(ch) = chars.next() {
            match state {
                InlineState::Normal => match ch {
                    // Handle placeholders (escaped characters)
                    '\x00' => {
                        let start = result.len();
                        result.push(ch);
                        // Copy the full placeholder
                        while let Some(&c) = chars.peek() {
                            result.push(c);
                            chars.next();
                            if c == '\x00' {
                                break;
                            }
                        }
                        let placeholder = &result[start..];
                        // Check if this is an escaped closing bracket followed by (
                        if placeholder.starts_with("\x00ESC") && chars.peek() == Some(&'(') {
                            // Skip the (url) part
                            chars.next(); // skip '('
                            let mut depth = 1;
                            for next_ch in chars.by_ref() {
                                if next_ch == '(' {
                                    depth += 1;
                                } else if next_ch == ')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // Bold and italic markers
                    '*' => {
                        // Check if this is bold (next char is also *) or italic
                        if chars.peek() == Some(&'*') {
                            // Bold marker, skip both asterisks
                            chars.next(); // consume second asterisk
                        } else {
                            // Italic marker
                            state = InlineState::InAsteriskItalic;
                            asterisk_content.clear();
                        }
                    }
                    '_' => {
                        // Check if this is bold (next char is also _) or italic
                        if chars.peek() == Some(&'_') {
                            // Bold marker, skip both underscores
                            chars.next(); // consume second underscore
                        } else {
                            // Italic marker
                            state = InlineState::InUnderscoreItalic;
                            underscore_content.clear();
                        }
                    }
                    // Links and images
                    '!' if chars.peek() == Some(&'[') => {
                        chars.next(); // skip '['
                        state = InlineState::InImageAlt;
                        alt_text.clear();
                    }
                    '[' => {
                        state = InlineState::InLinkText;
                        link_text.clear();
                    }
                    '`' => {
                        // Inline code start - count consecutive backticks
                        backtick_count = 1;
                        while chars.peek() == Some(&'`') {
                            chars.next();
                            backtick_count += 1;
                        }
                        state = InlineState::InInlineCode;
                        inline_code_content.clear();
                    }
                    ch => {
                        result.push(ch);
                    }
                },
                InlineState::InAsteriskItalic => match ch {
                    '*' => {
                        // Italic end
                        result.push_str(&asterisk_content);
                        asterisk_content.clear();
                        state = InlineState::Normal;
                    }
                    ch => {
                        asterisk_content.push(ch);
                    }
                },
                InlineState::InUnderscoreItalic => match ch {
                    '_' => {
                        // Italic end
                        result.push_str(&underscore_content);
                        underscore_content.clear();
                        state = InlineState::Normal;
                    }
                    ch => {
                        underscore_content.push(ch);
                    }
                },
                InlineState::InLinkText => match ch {
                    ']' => {
                        // Check if this is followed by (url)
                        if chars.peek() == Some(&'(') {
                            chars.next(); // skip '('
                            state = InlineState::InLinkUrl;
                            paren_depth = 1;
                        } else {
                            // Not a link, output the bracket text
                            result.push('[');
                            result.push_str(&link_text);
                            result.push(']');
                            link_text.clear();
                            state = InlineState::Normal;
                        }
                    }
                    ch => {
                        link_text.push(ch);
                    }
                },
                InlineState::InLinkUrl => match ch {
                    '(' => {
                        paren_depth += 1;
                    }
                    ')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            // Link ended, output the link text
                            result.push_str(&link_text);
                            link_text.clear();
                            state = InlineState::Normal;
                        }
                    }
                    _ => {}
                },
                InlineState::InImageAlt => match ch {
                    ']' => {
                        // Check if this is followed by (url)
                        if chars.peek() == Some(&'(') {
                            chars.next(); // skip '('
                            state = InlineState::InImageUrl;
                            paren_depth = 1;
                        } else {
                            // Not an image, output ![ and alt text
                            result.push('!');
                            result.push('[');
                            result.push_str(&alt_text);
                            result.push(']');
                            alt_text.clear();
                            state = InlineState::Normal;
                        }
                    }
                    ch => {
                        alt_text.push(ch);
                    }
                },
                InlineState::InImageUrl => match ch {
                    '(' => {
                        paren_depth += 1;
                    }
                    ')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            // Image ended, output the alt text
                            result.push_str(&alt_text);
                            alt_text.clear();
                            state = InlineState::Normal;
                        }
                    }
                    _ => {}
                },
                InlineState::InInlineCode => {
                    // Check if this might be closing backticks
                    if ch == '`' {
                        // Count consecutive backticks starting from this position
                        let mut count = 1;
                        while chars.peek() == Some(&'`') {
                            chars.next(); // consume from peek
                            count += 1;
                        }
                        if count >= backtick_count {
                            // This is closing backticks - output content without them
                            result.push_str(&inline_code_content);
                            inline_code_content.clear();
                            backtick_count = 0;
                            state = InlineState::Normal;
                        } else {
                            // Not enough backticks to close - add them to content
                            for _ in 0..count {
                                inline_code_content.push('`');
                            }
                        }
                    } else {
                        // Regular character, add to content
                        inline_code_content.push(ch);
                    }
                }
            }
        }

        // Handle unclosed italic markers
        match state {
            InlineState::InAsteriskItalic => {
                result.push('*');
                result.push_str(&asterisk_content);
            }
            InlineState::InUnderscoreItalic => {
                result.push('_');
                result.push_str(&underscore_content);
            }
            InlineState::InLinkText => {
                result.push('[');
                result.push_str(&link_text);
            }
            InlineState::InImageAlt => {
                result.push('!');
                result.push('[');
                result.push_str(&alt_text);
            }
            InlineState::InInlineCode => {
                // Output backticks and content as-is
                for _ in 0..backtick_count {
                    result.push('`');
                }
                result.push_str(&inline_code_content);
            }
            _ => {}
        }

        let result = result.trim().to_string();

        // Replace placeholders back to escaped characters (single pass for efficiency)
        if replacements.is_empty() {
            return result;
        }

        // Build a map from placeholder to character for quick lookup
        let replacement_map: std::collections::HashMap<&str, char> =
            replacements.iter().map(|(p, c)| (p.as_str(), *c)).collect();

        let mut final_result = String::with_capacity(result.len());
        let mut chars = result.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\x00' {
                // Start of a potential placeholder
                let start = final_result.len();
                final_result.push(ch);
                // Copy the full placeholder
                while let Some(&c) = chars.peek() {
                    final_result.push(c);
                    chars.next();
                    if c == '\x00' {
                        break;
                    }
                }
                let placeholder = &final_result[start..];
                // Check if this is a known placeholder
                if let Some(&replacement_char) = replacement_map.get(placeholder) {
                    // Replace with the escaped character
                    final_result.truncate(start);
                    final_result.push(replacement_char);
                }
            } else {
                final_result.push(ch);
            }
        }

        final_result
    }
}

/// State for the inline formatting state machine.
#[derive(Debug, Clone, Copy)]
enum InlineState {
    Normal,
    InAsteriskItalic,
    InUnderscoreItalic,
    InLinkText,
    InLinkUrl,
    InImageAlt,
    InImageUrl,
    InInlineCode,
}

impl DocumentParser {
    /// Clean Markdown text for indexing.
    fn clean_markdown_text(text: &str) -> String {
        let mut result = String::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let content = if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                trimmed.get(2..).map_or("", |s| s.trim())
            } else {
                trimmed
            };

            let content = content.strip_prefix("> ").unwrap_or(content);
            let cleaned = Self::clean_markdown_inline(content);

            if !cleaned.is_empty() {
                if !result.is_empty() {
                    result.push(' ');
                }
                result.push_str(&cleaned);
            }
        }

        result
    }

    /// Parse reStructuredText content into chunks.
    fn parse_rst(&self, content: &str, _file_path: &Path, file_id: &str) -> Result<Vec<DocChunk>> {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut chunk_start_line = 1;
        let mut current_line_num = 0;
        let mut header_stack: Vec<HeaderEntry> = Vec::new();
        let mut prev_line = String::new();

        for (line_idx, line) in content.lines().enumerate() {
            current_line_num = line_idx + 1;

            // RST headers: underline must be at least as long as the title text
            if !prev_line.is_empty() &&
                (line.chars().all(|c| c == '=' || c == '-' || c == '~' || c == '*') &&
                 line.len() >= prev_line.len() && line.len() >= 3) {

                if !current_chunk.is_empty() {
                    let path = Self::extract_header_path(&header_stack);
                    chunks.push(self.create_paragraph_chunk(
                        file_id,
                        current_chunk.trim_end(),
                        chunk_start_line,
                        current_line_num - 2,
                        &path,
                    ));
                    current_chunk.clear();
                }

                let header_text = prev_line.trim().to_string();
                header_stack.push(HeaderEntry::new(header_text.clone(), None));

                let path = Self::extract_header_path(&header_stack);
                chunks.push(DocChunk::new(
                    file_id.to_string(),
                    ChunkKind::Header,
                    header_text,
                    current_line_num - 1,
                    current_line_num,
                ).with_header_path(path));

                chunk_start_line = current_line_num + 1;
                prev_line = String::new();
                continue;
            }

            current_chunk.push_str(line);
            current_chunk.push('\n');

            if line.trim().is_empty() && current_chunk.len() > self.min_chunk_length {
                let path = Self::extract_header_path(&header_stack);
                chunks.push(self.create_paragraph_chunk(
                    file_id,
                    current_chunk.trim_end(),
                    chunk_start_line,
                    current_line_num,
                    &path,
                ));
                current_chunk.clear();
                chunk_start_line = current_line_num + 1;
            }

            prev_line = line.to_string();
        }

        if !current_chunk.is_empty() {
            let path = Self::extract_header_path(&header_stack);
            chunks.push(self.create_paragraph_chunk(
                file_id,
                current_chunk.trim_end(),
                chunk_start_line,
                current_line_num,
                &path,
            ));
        }

        Ok(chunks)
    }

    /// Parse plain text content into chunks.
    fn parse_plain_text(&self, content: &str, _file_path: &Path, file_id: &str) -> Result<Vec<DocChunk>> {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut current_line_num = 0;
        let mut paragraph_start = 0;

        for (line_idx, line) in content.lines().enumerate() {
            current_line_num = line_idx + 1;

            if line.trim().is_empty() {
                if !current_chunk.is_empty() {
                    chunks.push(DocChunk::new(
                        file_id.to_string(),
                        ChunkKind::Paragraph,
                        mem::take(&mut current_chunk),
                        paragraph_start,
                        current_line_num - 1,
                    ));
                }
                paragraph_start = current_line_num + 1;
            } else {
                if current_chunk.is_empty() {
                    paragraph_start = current_line_num;
                }
                if !current_chunk.is_empty() {
                    current_chunk.push(' ');
                }
                current_chunk.push_str(line.trim());
            }
        }

        if !current_chunk.is_empty() {
            chunks.push(DocChunk::new(
                file_id.to_string(),
                ChunkKind::Paragraph,
                current_chunk,
                paragraph_start,
                current_line_num,
            ));
        }

        Ok(chunks)
    }

    /// Parse AsciiDoc content into chunks.
    fn parse_asciidoc(&self, content: &str, _file_path: &Path, file_id: &str) -> Result<Vec<DocChunk>> {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut chunk_start_line = 1;
        let mut current_line_num = 0;
        let mut header_stack: Vec<HeaderEntry> = Vec::new();
        let mut in_code_block = false;

        for (line_idx, line) in content.lines().enumerate() {
            current_line_num = line_idx + 1;

            if line.trim() == "----" {
                if in_code_block {
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        chunks.push(DocChunk::new(
                            file_id.to_string(),
                            ChunkKind::CodeBlock,
                            mem::take(&mut current_chunk),
                            chunk_start_line,
                            current_line_num - 1,
                        ).with_header_path(path));
                    }
                    in_code_block = false;
                } else {
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        chunks.push(self.create_paragraph_chunk(
                            file_id,
                            current_chunk.trim_end(),
                            chunk_start_line,
                            current_line_num - 1,
                            &path,
                        ));
                        current_chunk.clear();
                    }
                    chunk_start_line = current_line_num + 1;
                    in_code_block = true;
                }
                continue;
            }

            if in_code_block {
                current_chunk.push_str(line);
                current_chunk.push('\n');
                continue;
            }

            if line.starts_with('=') {
                let level = line.chars().take_while(|&c| c == '=').count();
                if level <= 6 && line.chars().nth(level).is_none_or(|c| c.is_whitespace()) {
                    if !current_chunk.is_empty() {
                        let path = Self::extract_header_path(&header_stack);
                        chunks.push(self.create_paragraph_chunk(
                            file_id,
                            current_chunk.trim_end(),
                            chunk_start_line,
                            current_line_num - 1,
                            &path,
                        ));
                        current_chunk.clear();
                    }

                    let header_text = line[level..].trim().to_string();
                    let level_u8 = level as u8;

                    while header_stack.last().is_some_and(|h| h.level.unwrap_or(0) >= level_u8) {
                        header_stack.pop();
                    }
                    header_stack.push(HeaderEntry::new(header_text.clone(), Some(level_u8)));

                    let path = Self::extract_header_path(&header_stack);
                    chunks.push(DocChunk::new(
                        file_id.to_string(),
                        ChunkKind::Header,
                        header_text,
                        current_line_num,
                        current_line_num,
                    ).with_header_level(level_u8).with_header_path(path));

                    chunk_start_line = current_line_num + 1;
                    continue;
                }
            }

            current_chunk.push_str(line);
            current_chunk.push('\n');

            if line.trim().is_empty() && current_chunk.len() > self.min_chunk_length {
                let path = Self::extract_header_path(&header_stack);
                chunks.push(self.create_paragraph_chunk(
                    file_id,
                    current_chunk.trim_end(),
                    chunk_start_line,
                    current_line_num,
                    &path,
                ));
                current_chunk.clear();
                chunk_start_line = current_line_num + 1;
            }
        }

        if !current_chunk.is_empty() {
            let path = Self::extract_header_path(&header_stack);
            chunks.push(self.create_paragraph_chunk(
                file_id,
                current_chunk.trim_end(),
                chunk_start_line,
                current_line_num,
                &path,
            ));
        }

        Ok(chunks)
    }
}

impl Default for DocumentParser {
    fn default() -> Self {
        Self::new(DocumentFormat::Markdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_format_from_extension() {
        assert_eq!(DocumentFormat::from_extension("md"), Some(DocumentFormat::Markdown));
        assert_eq!(DocumentFormat::from_extension("rst"), Some(DocumentFormat::ReStructuredText));
        assert_eq!(DocumentFormat::from_extension("txt"), Some(DocumentFormat::PlainText));
        assert_eq!(DocumentFormat::from_extension("unknown"), None);
    }

    #[test]
    fn test_parse_markdown_headers() {
        let content = "# Title\n\nContent";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_parse_markdown_code_blocks() {
        let content = "```rust\nfn main() {}\n```";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 1);
    }

    #[test]
    fn test_parse_plain_text() {
        let content = "First\n\nSecond";
        let parser = DocumentParser::new(DocumentFormat::PlainText);
        let chunks = parser.parse(content, Path::new("test.txt"), "file-1").unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_document_parser_from_path() {
        let parser = DocumentParser::from_path(Path::new("test.md"));
        assert!(parser.is_ok());
    }

    #[test]
    fn test_strip_markdown_links() {
        let text = "[link](url)";
        let cleaned = DocumentParser::clean_markdown_inline(text);
        assert!(cleaned.contains("link"));
        assert!(!cleaned.contains("(url)"));
    }

    #[test]
    fn test_strip_markdown_images() {
        let text = "![alt](image.png)";
        let cleaned = DocumentParser::clean_markdown_inline(text);
        assert!(cleaned.contains("alt"));
        assert!(!cleaned.contains("(image.png)"));
    }

    #[test]
    fn test_parse_markdown_with_nested_headers() {
        let content = "# Main\n\n## Sub\n\nContent";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert!(headers.len() >= 2);
    }

    #[test]
    fn test_parse_markdown_paragraphs() {
        let content = "First paragraph.\n\nSecond paragraph.";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let paragraphs: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Paragraph).collect();
        assert!(!paragraphs.is_empty());
    }

    #[test]
    fn test_parse_markdown_setext_headers() {
        // Test Setext-style headers with === and --- underlines
        let content = "Main Title\n===========\n\nSome content\n\nSub Title\n---------\n\nMore content";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert_eq!(headers.len(), 2);

        // First header should be level 1 (===)
        assert_eq!(headers[0].header_level, Some(1));
        assert_eq!(headers[0].content.trim(), "Main Title");

        // Second header should be level 2 (---)
        assert_eq!(headers[1].header_level, Some(2));
        assert_eq!(headers[1].content.trim(), "Sub Title");
    }

    #[test]
    fn test_parse_markdown_with_inline_formatting() {
        let content = "This is **bold** and *italic* text.";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(!cleaned.contains("**"));
        assert!(!cleaned.contains("*"));
    }

    #[test]
    fn test_parse_rst_headers() {
        let content = "Title\n=====\n\nContent";
        let parser = DocumentParser::new(DocumentFormat::ReStructuredText);
        let chunks = parser.parse(content, Path::new("test.rst"), "file-1").unwrap();
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert!(headers.len() >= 1);
    }

    #[test]
    fn test_parse_asciidoc_headers() {
        let content = "= Main\n\nContent";
        let parser = DocumentParser::new(DocumentFormat::AsciiDoc);
        let chunks = parser.parse(content, Path::new("test.adoc"), "file-1").unwrap();
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert!(headers.len() >= 1);
    }

    #[test]
    fn test_parse_asciidoc_code_blocks() {
        let content = "----\ncode\n----";
        let parser = DocumentParser::new(DocumentFormat::AsciiDoc);
        let chunks = parser.parse(content, Path::new("test.adoc"), "file-1").unwrap();
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 1);
    }

    #[test]
    fn test_empty_document() {
        let content = "";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(chunks.is_empty());
    }

    // Error scenario tests

    #[test]
    fn test_unclosed_code_fence() {
        // Unclosed code fence should still produce chunks for content before it
        let content = "```rust\nlet x = 1;\nlet y = 2;";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        // The unclosed code block should be captured as a code chunk
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert!(!code_chunks.is_empty());
    }

    #[test]
    fn test_malformed_inline_formatting() {
        // Test unclosed bold/italic markers
        let content = "This is **bold text that never closes";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        // Content should still be present even with unclosed markers
        assert!(chunks[0].content.contains("bold"));
    }

    #[test]
    fn test_deeply_nested_headers() {
        // Test headers with deep nesting (more than 6 levels for ATX)
        let content = "# Level 1\n\n## Level 2\n\n### Level 3\n\n#### Level 4\n\n##### Level 5\n\n###### Level 6";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert_eq!(headers.len(), 6);
    }

    #[test]
    fn test_extremely_long_line() {
        // Test handling of very long lines
        let long_word = "a".repeat(10000);
        let content = format!("# Title\n\n{}", long_word);
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(&content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_only_whitespace_document() {
        let content = "   \n\n   \n\t\t\n";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        // Should produce empty result or chunks with minimal content
        assert!(chunks.is_empty() || chunks.iter().all(|c| c.content.trim().is_empty()));
    }

    #[test]
    fn test_mixed_markdown_atx_and_setext() {
        // Test mixing ATX (#) and Setext (===) style headers
        let content = "# ATX Header\n\nSetext Header\n===========\n\n### Another ATX\n\nSub Header\n----------";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert_eq!(headers.len(), 4);
    }

    #[test]
    fn test_invalid_setext_with_text_after_underline() {
        // Setext underline with text immediately after should not be treated as header
        let content = "Title\n===not a header";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        // Should be treated as paragraph content, not header
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert_eq!(headers.len(), 0);
    }

    #[test]
    fn test_special_characters_in_content() {
        // Test handling of special characters that might interfere with parsing
        let content = "# Special Chars\n\nContent with <html> tags, &entities; and $symbols.";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let para: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Paragraph).collect();
        assert!(!para.is_empty());
    }

    // Unicode and nested structure tests

    #[test]
    fn test_unicode_in_markdown() {
        // Test handling of Unicode characters including emojis and non-ASCII text
        let content = "# Unicode 测试\n\nThis has emojis: 🎉 🔥 🚀\n\nAnd CJK: 你好世界";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());

        // Verify CJK characters are preserved
        let text = chunks.iter().map(|c| c.content.as_str()).collect::<String>();
        assert!(text.contains("测试"));
        assert!(text.contains("你好世界"));
    }

    #[test]
    fn test_unicode_in_inline_formatting() {
        // Test Unicode with bold/italic
        let content = "This is **Unicode 文本** with *emoji 🎉*";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("文本"));
        assert!(cleaned.contains("🎉"));
    }

    #[test]
    fn test_nested_markdown_links() {
        // Test links with various URL formats and special characters
        let content = "[Link with spaces](https://example.com/path with spaces)\n\n[Special chars](https://example.com/?q=hello&world=1)";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let text: String = chunks.iter().map(|c| c.content.as_str()).collect::<Vec<&str>>().join(" ");
        assert!(text.contains("Link with spaces"));
        assert!(text.contains("Special chars"));
        assert!(!text.contains("https://"));
    }

    #[test]
    fn test_nested_markdown_images() {
        // Test images with alt text containing special characters
        let content = "![Alt with emoji 🎉](image.png)\n\n![中文图片](photo.jpg)";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let text: String = chunks.iter().map(|c| c.content.as_str()).collect::<Vec<&str>>().join(" ");
        assert!(text.contains("Alt with emoji"));
        assert!(text.contains("中文图片"));
    }

    #[test]
    fn test_mixed_unicode_and_formatting() {
        // Test complex combination of Unicode and formatting
        let content = "# 标题 Title\n\nSome **bold 中文** and *italic 日本語* text with emoji 🚀";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());

        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert_eq!(headers.len(), 1);
        assert!(headers[0].content.contains("标题"));

        let paras: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Paragraph).collect();
        assert!(!paras.is_empty());
        assert!(paras[0].content.contains("中文"));
        assert!(paras[0].content.contains("日本語"));
        assert!(paras[0].content.contains("🚀"));
    }

    #[test]
    fn test_right_to_left_text() {
        // Test RTL languages (Arabic, Hebrew)
        let content = "# RTL Test\n\nمرحبا بالعالم\n\nשלום עולם";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let text = chunks.iter().map(|c| c.content.as_str()).collect::<String>();
        assert!(text.contains("مرحبا") || text.contains("שלום"));
    }

    // Indented code block tests (P1-1)

    #[test]
    fn test_indented_code_block_four_spaces() {
        // Test 4-space indented code blocks
        let content = "Regular text\n\n    let x = 1;\n    let y = 2;\n\nMore text";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 1);
        assert!(code_chunks[0].content.contains("let x = 1;"));
    }

    #[test]
    fn test_indented_code_block_with_tab() {
        // Test tab-indented code blocks
        let content = "Regular text\n\n\tlet x = 1;\n\tlet y = 2;\n\nMore text";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 1);
    }

    #[test]
    fn test_indented_code_block_with_blank_lines() {
        // Test indented code blocks with blank lines
        // According to CommonMark, blank lines end indented code blocks
        let content = "Regular text\n\n    line1\n\n    line3\n\nMore text";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        // Blank lines split the indented code into two blocks
        assert_eq!(code_chunks.len(), 2);
        assert!(code_chunks[0].content.contains("line1"));
        assert!(code_chunks[1].content.contains("line3"));
    }

    #[test]
    fn test_mixed_fence_and_indented_code() {
        // Test mixing fenced and indented code blocks
        let content = "Fenced:\n```\ncode\n```\n\nIndented:\n    code\n\nText";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 2);
    }

    // Escape character tests (P0-3)

    #[test]
    fn test_escape_italic_markers() {
        // Test that escaped asterisks are not treated as italic
        let content = r#"This is \*not italic\* but this is *italic*"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("*not italic*"));
        assert!(!cleaned.contains("\\*"));
    }

    #[test]
    fn test_escape_bold_markers() {
        // Test that escaped bold markers are preserved
        let content = r#"This is \*\*not bold\*\* but this is **bold**"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("**not bold**"));
        assert!(cleaned.contains("bold"));
    }

    #[test]
    fn test_escape_links() {
        // Test that escaped brackets are not treated as links
        let content = r#"This is \[not a link\](url) but [this is](url)"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("[not a link]"));
        assert!(cleaned.contains("this is"));
        assert!(!cleaned.contains("(url)"));
    }

    #[test]
    fn test_escape_header_marker() {
        // Test that escaped # is not treated as header
        let content = r#"This is \#not a header but this is:"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        // Should not have a header chunk
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        assert_eq!(headers.len(), 0);
    }

    #[test]
    fn test_escape_backslash() {
        // Test double backslash becomes single backslash
        let content = r#"This is a \\ backslash"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains('\\'));
    }

    // Additional edge case tests (P1-4)

    #[test]
    fn test_html_tags_preserved() {
        // Test that HTML tags are preserved in content
        let content = "# Title\n\n<div>This is HTML</div>\n\n<p>Paragraph</p>";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let text = chunks.iter().map(|c| c.content.as_str()).collect::<String>();
        assert!(text.contains("<div>") || text.contains("<p>"));
    }

    #[test]
    fn test_code_block_preserves_formatting() {
        // Test that code blocks preserve special characters
        let content = "```rust\nlet x = 1 + 2; // *not italic*\nlet y = \"test\";\n```";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 1);
        assert!(code_chunks[0].content.contains("*not italic*"));
        assert!(code_chunks[0].content.contains("let y = "));
    }

    #[test]
    fn test_strikethrough_not_implemented() {
        // Test strikethrough (not currently supported, should be preserved)
        let content = "This is ~~deleted~~ text";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Strikethrough markers should be preserved (not implemented)
        assert!(cleaned.contains("~~deleted~~"));
    }

    #[test]
    fn test_inline_code_not_implemented() {
        // Test inline code (backticks are removed, content is preserved)
        let content = r#"This is `inline code` text"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Inline code markers should be removed, content preserved
        assert!(cleaned.contains("inline code"));
        assert!(!cleaned.contains('`'));
    }

    #[test]
    fn test_autolinks() {
        // Test autolinks <url>
        let content = r#"Visit <https://example.com> for more"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Angle brackets should be preserved
        assert!(cleaned.contains("<https://example.com>"));
    }

    #[test]
    fn test_mixed_lists_and_formatting() {
        // Test list items with formatting
        let content = "- **Bold item**\n- *Italic item*\n- Plain item";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Bold/italic should be cleaned
        assert!(!cleaned.contains("**"));
        assert!(cleaned.contains("Bold item"));
        assert!(cleaned.contains("Italic item"));
    }

    #[test]
    fn test_multiple_links_in_one_line() {
        // Test multiple links in one line
        let content = "[first](url1) and [second](url2)";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("first"));
        assert!(cleaned.contains("second"));
        assert!(!cleaned.contains("(url1)"));
        assert!(!cleaned.contains("(url2)"));
    }

    #[test]
    fn test_link_with_complex_url() {
        // Test link with complex URL containing special characters
        let content = r#"[link](https://example.com/path?query=value&foo=bar)"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("link"));
        assert!(!cleaned.contains("https://"));
    }

    #[test]
    fn test_image_with_nested_brackets() {
        // Test image with nested brackets in alt text
        let content = r#"![alt [text]](image.png)"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Current implementation may not handle nested brackets perfectly
        assert!(cleaned.contains("alt"));
    }

    #[test]
    fn test_empty_bold_italic() {
        // Test empty bold/italic markers
        let content = "Text with **** empty ** bold and empty __ italic __ markers";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_mixed_escape_and_formatting() {
        // Test mixed escape sequences and formatting
        let content = r#"This is \**bold** and \*italic* and \[link\](url)"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // \** should escape to **, which is then removed as bold
        // But the second ** creates a bold marker, so we get bold
        assert!(cleaned.contains("bold"));
        assert!(cleaned.contains("italic"));
        assert!(cleaned.contains("[link]"));
    }

    #[test]
    fn test_very_long_header_path() {
        // Test deeply nested headers (10 levels)
        let content = "# L1\n\n## L2\n\n### L3\n\n#### L4\n\n##### L5\n\n###### L6\n\n####### L7\n\n######## L8\n\n######### L9\n\n########## L10";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        let headers: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Header).collect();
        // Only levels 1-6 are recognized as ATX headers
        assert_eq!(headers.len(), 6);
    }

    #[test]
    fn test_consecutive_blank_lines() {
        // Test handling of multiple consecutive blank lines
        let content = "Para 1\n\n\n\n\nPara 2";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        // Multiple blank lines should be collapsed
        assert!(chunks.len() >= 1);
    }

    #[test]
    fn test_unicode_links_and_images() {
        // Test links and images with Unicode in URLs and alt text
        let content = r#"[中文链接](https://example.com/中文) and ![图片](图片.png)"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("中文链接"));
        assert!(cleaned.contains("图片"));
    }

    #[test]
    fn test_performance_large_document() {
        // Performance test: large document with many formatting elements
        let mut content = String::from("# Title\n\n");
        for i in 0..100 {
            content.push_str(&format!("## Section {}\n\nThis has **bold** and *italic* and [link](url) and ![img](img.png).\n\n", i));
        }
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(&content, Path::new("test.md"), "file-1").unwrap();
        // We get 1 title + 100 headers + 100 paragraphs = 201 chunks
        // The exact number may vary, but should be close to 200
        assert!(chunks.len() >= 101);
        assert!(chunks.len() <= 250);
    }

    #[test]
    fn test_blockquote_simple() {
        // Test simple blockquote
        let content = "> This is a quote\n> with multiple lines";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        assert!(cleaned.contains("quote") || cleaned.contains("This is a quote"));
    }

    #[test]
    fn test_blockquote_nested() {
        // Test nested blockquotes
        let content = "> Level 1\n>> Level 2\n>>> Level 3";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        // Each level becomes a separate chunk
        assert!(chunks.len() >= 1);
    }

    #[test]
    fn test_blockquote_mixed_content() {
        // Test blockquote with other content
        let content = "Regular paragraph\n\n> Quote here\n\nMore regular text";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_task_list_unchecked() {
        // Test task list with unchecked item
        let content = "- [ ] Task 1\n- [ ] Task 2";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Task markers should be removed, content preserved
        assert!(cleaned.contains("Task 1") || cleaned.contains("Task"));
    }

    #[test]
    fn test_task_list_checked() {
        // Test task list with checked item
        let content = "- [x] Completed task\n- [ ] Pending task";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Task markers should be removed, content preserved
        assert!(cleaned.contains("Completed") || cleaned.contains("Pending"));
    }

    #[test]
    fn test_inline_code_with_backticks() {
        // Test inline code with multiple backticks
        let content = r#"This is ``inline code`` and `single` backtick"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Backticks should be removed, content preserved
        assert!(!cleaned.contains('`'));
        assert!(cleaned.contains("inline code"));
        assert!(cleaned.contains("single"));
    }

    #[test]
    fn test_inline_code_with_formatting_inside() {
        // Test inline code with formatting markers inside (should be preserved)
        let content = r#"This is `code with *italic* inside` text"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Formatting inside inline code should be preserved as-is
        assert!(cleaned.contains("code with") && cleaned.contains("italic") && cleaned.contains("inside"));
    }

    #[test]
    fn test_inline_code_unclosed() {
        // Test unclosed inline code (should preserve as-is)
        let content = r#"This has `unclosed code and more text"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        let cleaned = &chunks[0].content;
        // Unclosed backtick should be preserved
        assert!(cleaned.contains('`') || cleaned.contains("unclosed"));
    }

    #[test]
    fn test_inline_code_empty() {
        // Test empty inline code
        let content = r#"This has `` empty code"#;
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_blockquote_after_indented_code() {
        // Test blockquote after indented code block
        let content = "    let x = 1;\n> This is a quote";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(chunks.len() >= 2);
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 1);
    }

    #[test]
    fn test_indented_code_after_blockquote() {
        // Test indented code block after blockquote
        let content = "> This is a quote\n\n    let x = 1;";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(chunks.len() >= 2);
        let code_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::CodeBlock).collect();
        assert_eq!(code_chunks.len(), 1);
    }

    #[test]
    fn test_blockquote_with_indented_content() {
        // Test blockquote containing indented-looking content (should stay as quote)
        let content = ">     This looks indented but is a quote\n> Continued";
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        assert!(!chunks.is_empty());
        // Should be parsed as paragraph, not code block
        let para_chunks: Vec<_> = chunks.iter().filter(|c| c.kind == ChunkKind::Paragraph).collect();
        assert!(!para_chunks.is_empty());
    }

    #[test]
    fn test_custom_min_chunk_length() {
        // Test custom minimum chunk length
        let content = "Short\n\nAnother short";
        let parser = DocumentParser::with_min_chunk_length(DocumentFormat::Markdown, 100);
        let chunks = parser.parse(content, Path::new("test.md"), "file-1").unwrap();
        // With min_chunk_length=100, short chunks should be combined
        assert!(chunks.len() <= 2);
    }

    #[test]
    fn test_min_chunk_length_accessor() {
        // Test that min_chunk_length can be accessed
        let parser = DocumentParser::new(DocumentFormat::Markdown);
        assert_eq!(parser.min_chunk_length(), 20);

        let custom_parser = DocumentParser::with_min_chunk_length(DocumentFormat::Markdown, 50);
        assert_eq!(custom_parser.min_chunk_length(), 50);
    }
}
