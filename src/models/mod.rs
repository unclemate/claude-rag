//! Data models for Claude RAG.
//!
//! Defines all core data structures used throughout the application.

pub mod chunk;
pub mod commit;
pub mod diff;
pub mod file;
pub mod message;
pub mod session;
pub mod symbol;

pub use chunk::{ChunkKind, DocChunk};
pub use commit::Commit;
pub use diff::GitDiff;
pub use file::File;
pub use message::{Message, Role};
pub use session::Session;
pub use symbol::{Symbol, SymbolKind};

/// Content type enum for indexed items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ContentType {
    /// Session record.
    Session,
    /// Individual message within a session.
    Message,
    /// Source file.
    File,
    /// Code symbol (function, class, etc.).
    Symbol,
    /// Git commit.
    Commit,
    /// Git diff.
    GitDiff,
}
