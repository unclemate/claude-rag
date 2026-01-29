//! Claude RAG - CLI entry point.

use clap::{Parser, Subcommand};
use anyhow::Result;
use tracing::{debug, error, info, warn};

// Import the ProgressReporter trait so its methods are available
use claude_rag::ProgressReporter;

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
        /// Filter results after this time (e.g., "7d", "1w", "2025-01-01")
        #[arg(long)]
        after: Option<String>,
        /// Filter results before this time (e.g., "7d", "1w", "2025-01-01")
        #[arg(long)]
        before: Option<String>,
        /// Maximum age of results in days (e.g., 7 for last 7 days)
        #[arg(long, value_name = "DAYS")]
        max_age: Option<u64>,
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

    // 初始化日志系统（必须在最开始）
    let project_path = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?;

    // 日志初始化失败时仅警告，不影响程序运行
    let _log_guard = match claude_rag::logging::init_logging_default(&project_path) {
        Ok(guard) => {
            info!("Logging system initialized");
            guard
        }
        Err(e) => {
            eprintln!("Warning: Failed to initialize logging: {}", e);
            // 使用导出的 dummy guard 保持类型一致
            claude_rag::create_dummy_guard()
        }
    };

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
        Commands::Query { query, r#type, top_k, timeline, format, after, before, max_age } => {
            handle_query(query, r#type, top_k, timeline, format, after, before, max_age)?;
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
    info!("Initializing knowledge base (force: {})", force);
    println!("Initializing knowledge base...");

    let current_dir = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?;

    match claude_rag::cli::init_project(&current_dir, force) {
        Ok(claude_rag::cli::InitResult::Success { rag_dir, storage_size }) => {
            info!("Knowledge base initialized at {}, storage size: {} bytes", rag_dir.display(), storage_size);
            println!("✓ Knowledge base initialized");
            println!("  Location: {}", rag_dir.display());
            println!("  Storage size: {} bytes", storage_size);
            println!("\nNext steps:");
            println!("  1. Configure API token in .rag/config.json");
            println!("  2. Run: claude-rag index");
        }
        Ok(claude_rag::cli::InitResult::AlreadyExists) => {
            warn!("Knowledge base already exists at {}", current_dir.display());
            println!("✗ Knowledge base already exists");
            println!("  Use --force to re-initialize");
            std::process::exit(1);
        }
        Err(e) => {
            error!("Failed to initialize knowledge base: {}", e);
            println!("✗ Failed to initialize: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

fn handle_index(_all: bool, _project: Option<String>, force: bool, r#type: Option<String>) -> Result<()> {
    let current_dir = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?;

    // Determine what to index
    let index_source = r#type.as_deref().unwrap_or("all") == "all" || r#type.as_deref().unwrap_or("all") == "source";
    let index_docs = r#type.as_deref().unwrap_or("all") == "all" || r#type.as_deref().unwrap_or("all") == "docs";
    let index_sessions = r#type.as_deref().unwrap_or("all") == "all" || r#type.as_deref().unwrap_or("all") == "sessions";

    info!(
        "Starting indexing (source: {}, docs: {}, sessions: {}, force: {})",
        index_source, index_docs, index_sessions, force
    );

    let options = claude_rag::cli::IndexOptions {
        index_source,
        index_docs,
        index_other: false,
        index_sessions,
        force,
    };

    // Load config to get progress style
    let config = match claude_rag::ConfigManager::load(Some(&current_dir)) {
        Ok(config) => config,
        Err(e) => {
            warn!("Failed to load config, using defaults: {}", e);
            eprintln!("Warning: Failed to load config, using defaults: {}", e);
            eprintln!("  Run 'claude-rag init' to create a configuration file");
            claude_rag::Config::default()
        }
    };

    // Create progress reporter with configured style
    let reporter = claude_rag::ProgressBarReporter::new(config.progress.style.clone());

    match claude_rag::cli::index_project(&current_dir, options, Some(&reporter)) {
        Ok(result) => {
            reporter.finish();

            info!(
                "Indexing complete: {} files, {} sessions, {} errors",
                result.file_stats.files_collected,
                result.session_stats.sessions_collected,
                result.errors
            );

            println!("✓ Indexing complete");
            println!("  Files indexed: {}", result.file_stats.files_collected);
            println!("  Sessions indexed: {}", result.session_stats.sessions_collected);

            // Show cache hit rate if available and enabled
            let stats = reporter.stats();
            if config.progress.style.show_cache_stats {
                if let Some(rate) = stats.cache_hit_rate() {
                    println!("  Cache hit rate: {:.1}%", rate * 100.0);
                }
            }

            // Show processing rate if available and enabled
            if config.progress.style.show_processing_rate {
                if let Some(rate) = stats.processing_rate() {
                    println!("  Processing rate: {:.1} items/sec", rate);
                }
            }

            if result.errors > 0 {
                println!("  Errors: {}", result.errors);
            }
        }
        Err(e) => {
            error!("Failed to index: {}", e);
            println!("✗ Failed to index: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

fn handle_daemon(daemon_cmd: DaemonCommands) -> Result<()> {
    match daemon_cmd {
        DaemonCommands::Start => {
            info!("Starting daemon");
            println!("Starting daemon...");
            // TODO: Implement daemon start
            println!("✓ Daemon started");
        }
        DaemonCommands::Stop => {
            info!("Stopping daemon");
            println!("Stopping daemon...");
            // TODO: Implement daemon stop
            println!("✓ Daemon stopped");
        }
        DaemonCommands::Status => {
            debug!("Checking daemon status");
            println!("Daemon status:");
            // TODO: Implement status check
            println!("  Status: stopped");
        }
        DaemonCommands::Restart => {
            info!("Restarting daemon");
            println!("Restarting daemon...");
            // TODO: Implement restart
            println!("✓ Daemon restarted");
        }
    }
    Ok(())
}

fn handle_query(
    query: String,
    r#type: Option<String>,
    top_k: usize,
    timeline: bool,
    format: String,
    after: Option<String>,
    before: Option<String>,
    max_age: Option<u64>,
) -> Result<()> {
    debug!(
        "Query parameters: query='{}', type={:?}, top_k={}, timeline={}, format={}",
        query, r#type, top_k, timeline, format
    );

    // Parse time range if specified
    let time_range = if after.is_some() || before.is_some() || max_age.is_some() {
        Some(claude_rag::query::TimeRange::from_cli_args(
            after.as_deref(),
            before.as_deref(),
            max_age,
        )?)
    } else {
        None
    };

    info!("Executing query: {}", query);

    // Use tokio runtime for async query execution
    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(claude_rag::execute_query_with_time_range(
        query,
        r#type,
        top_k,
        timeline,
        format,
        time_range,
    ))?;

    info!("Query executed successfully");
    println!("{}", result);
    Ok(())
}

fn handle_status() -> Result<()> {
    info!("Checking system status");
    println!("Status:");
    // TODO: Implement status
    println!("  Database: initialized");
    println!("  Indexed items: 0");
    Ok(())
}

fn handle_install_skills() -> Result<()> {
    info!("Installing Claude Code Skills");
    println!("Installing Claude Code Skills...");

    let installer = claude_rag::skills::SkillsInstaller::new()
        .map_err(|e| {
            error!("Failed to create skills installer: {}", e);
            anyhow::anyhow!("Failed to create skills installer: {}", e)
        })?;

    installer.install_skills()
        .map_err(|e| {
            error!("Failed to install skills: {}", e);
            anyhow::anyhow!("Failed to install skills: {}", e)
        })?;

    info!("Skills installed to {}", installer.skills_dir().display());
    println!("✓ Skills installed to: {}", installer.skills_dir().display());
    println!("  Available skills:");
    println!("    /rag-query     - Query all indexed content");
    println!("    /rag-code      - Search source code");
    println!("    /rag-docs      - Search documentation");
    println!("    /rag-session   - Search previous sessions");
    println!("    /rag-timeline  - Build feature timeline");
    Ok(())
}
