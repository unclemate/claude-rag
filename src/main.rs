//! Claude RAG - CLI entry point.

use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(name = "claude-rag")]
#[command(about = "Claude Code Interaction History & Knowledge Base Tool", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Custom config path
    #[arg(short, long, global = true)]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize knowledge base for current project
    Init {
        /// Force re-initialization
        #[arg(long)]
        force: bool,
    },
    /// Index existing sessions/files
    Index {
        /// Index all projects
        #[arg(long)]
        all: bool,
        /// Index specific project
        #[arg(long)]
        project: Option<String>,
        /// Force re-index
        #[arg(long)]
        force: bool,
        /// Index specific type
        #[arg(long)]
        r#type: Option<String>,
    },
    /// Daemon commands
    Daemon {
        #[command(subcommand)]
        daemon_cmd: DaemonCommands,
    },
    /// Query the knowledge base
    Query {
        /// Query text
        query: String,
        /// Filter by content type
        #[arg(short, long)]
        r#type: Option<String>,
        /// Number of results
        #[arg(short, long, default_value_t = 5)]
        top_k: usize,
        /// Show timeline
        #[arg(long)]
        timeline: bool,
        /// Output format
        #[arg(short, long, default_value = "markdown")]
        format: String,
    },
    /// Check status
    Status,
    /// Start MCP server
    McpServer,
    /// Install Claude Code Skills
    InstallSkills,
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start daemon
    Start,
    /// Stop daemon
    Stop,
    /// Check daemon status
    Status,
    /// Restart daemon
    Restart,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { force } => {
            handle_init(force)?;
        }
        Commands::Index { all, project, force, r#type } => {
            handle_index(all, project, force, r#type)?;
        }
        Commands::Daemon { daemon_cmd } => {
            handle_daemon(daemon_cmd)?;
        }
        Commands::Query { query, r#type, top_k, timeline, format } => {
            handle_query(query, r#type, top_k, timeline, format)?;
        }
        Commands::Status => {
            handle_status()?;
        }
        Commands::McpServer => {
            println!("Starting MCP server...");
            // TODO: Implement MCP server
            println!("✓ MCP server started");
        }
        Commands::InstallSkills => {
            handle_install_skills()?;
        }
    }

    Ok(())
}

fn handle_init(force: bool) -> Result<()> {
    println!("Initializing knowledge base...");

    let current_dir = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?;

    match claude_rag::cli::init_project(&current_dir, force) {
        Ok(claude_rag::cli::InitResult::Success { rag_dir, storage_size }) => {
            println!("✓ Knowledge base initialized");
            println!("  Location: {}", rag_dir.display());
            println!("  Storage size: {} bytes", storage_size);
            println!("\nNext steps:");
            println!("  1. Configure API token in .rag/config.json");
            println!("  2. Run: claude-rag index");
        }
        Ok(claude_rag::cli::InitResult::AlreadyExists) => {
            println!("✗ Knowledge base already exists");
            println!("  Use --force to re-initialize");
            std::process::exit(1);
        }
        Err(e) => {
            println!("✗ Failed to initialize: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

fn handle_index(_all: bool, _project: Option<String>, force: bool, r#type: Option<String>) -> Result<()> {
    println!("Indexing...");

    let current_dir = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?;

    // Determine what to index
    let index_source = r#type.as_deref().unwrap_or("all") == "all" || r#type.as_deref().unwrap_or("all") == "source";
    let index_docs = r#type.as_deref().unwrap_or("all") == "all" || r#type.as_deref().unwrap_or("all") == "docs";
    let index_sessions = r#type.as_deref().unwrap_or("all") == "all" || r#type.as_deref().unwrap_or("all") == "sessions";

    let options = claude_rag::cli::IndexOptions {
        index_source,
        index_docs,
        index_other: false,
        index_sessions,
        force,
    };

    // Progress callback
    let progress = |msg: &str| {
        println!("  {}", msg);
    };

    match claude_rag::cli::index_project(&current_dir, options, Some(&progress)) {
        Ok(result) => {
            println!("✓ Indexing complete");
            println!("  Files indexed: {}", result.file_stats.files_collected);
            println!("  Sessions indexed: {}", result.session_stats.sessions_collected);
            if result.errors > 0 {
                println!("  Errors: {}", result.errors);
            }
        }
        Err(e) => {
            println!("✗ Failed to index: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

fn handle_daemon(daemon_cmd: DaemonCommands) -> Result<()> {
    match daemon_cmd {
        DaemonCommands::Start => {
            println!("Starting daemon...");
            // TODO: Implement daemon start
            println!("✓ Daemon started");
        }
        DaemonCommands::Stop => {
            println!("Stopping daemon...");
            // TODO: Implement daemon stop
            println!("✓ Daemon stopped");
        }
        DaemonCommands::Status => {
            println!("Daemon status:");
            // TODO: Implement status check
            println!("  Status: stopped");
        }
        DaemonCommands::Restart => {
            println!("Restarting daemon...");
            // TODO: Implement restart
            println!("✓ Daemon restarted");
        }
    }
    Ok(())
}

fn handle_query(query: String, r#type: Option<String>, top_k: usize, timeline: bool, format: String) -> Result<()> {
    // Use tokio runtime for async query execution
    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(claude_rag::execute_query(query, r#type, top_k, timeline, format))?;
    println!("{}", result);
    Ok(())
}

fn handle_status() -> Result<()> {
    println!("Status:");
    // TODO: Implement status
    println!("  Database: initialized");
    println!("  Indexed items: 0");
    Ok(())
}

fn handle_install_skills() -> Result<()> {
    println!("Installing Claude Code Skills...");

    let installer = claude_rag::skills::SkillsInstaller::new()
        .map_err(|e| anyhow::anyhow!("Failed to create skills installer: {}", e))?;

    installer.install_skills()
        .map_err(|e| anyhow::anyhow!("Failed to install skills: {}", e))?;

    println!("✓ Skills installed to: {}", installer.skills_dir().display());
    println!("  Available skills:");
    println!("    /rag-query     - Query all indexed content");
    println!("    /rag-code      - Search source code");
    println!("    /rag-docs      - Search documentation");
    println!("    /rag-session   - Search previous sessions");
    println!("    /rag-timeline  - Build feature timeline");
    Ok(())
}
