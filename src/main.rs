//! Claude RAG - CLI entry point.

use clap::{Parser, Subcommand};
use anyhow::Result;
use tracing::{debug, error, info, warn};
use std::fs;

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
    /// Index code symbols for semantic search
    IndexCode {
        /// Index all projects
        #[arg(long)]
        all: bool,
        /// Index specific project
        #[arg(long)]
        project: Option<String>,
        /// Git branch to index (default: current branch)
        #[arg(long)]
        branch: Option<String>,
        /// Force re-index
        #[arg(long)]
        force: bool,
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
        Commands::IndexCode { all, project, branch, force } => {
            handle_index_code(all, project, branch, force)?;
        }
        Commands::Daemon { daemon_cmd } => {
            handle_daemon(daemon_cmd).await?;
        }
        Commands::Query { query, r#type, top_k, timeline, format, after, before, max_age } => {
            handle_query(query, r#type, top_k, timeline, format, after, before, max_age)?;
        }
        Commands::Status => {
            handle_status()?;
        }
        Commands::McpServer => {
            info!("Starting MCP server");

            // 创建 MCP 服务器实例
            let server = match claude_rag::mcp::McpServer::new(None) {
                Ok(server) => server,
                Err(e) => {
                    error!("Failed to create MCP server: {}", e);
                    eprintln!("✗ Failed to create MCP server: {}", e);
                    std::process::exit(1);
                }
            };

            // 运行服务器（监听 stdin/stdout）
            if let Err(e) = server.run().await {
                error!("MCP server error: {}", e);
                eprintln!("✗ MCP server error: {}", e);
                std::process::exit(1);
            }

            info!("MCP server shutdown");
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

/// Handle code symbol indexing command
///
/// This function indexes code symbols for semantic search.
fn handle_index_code(_all: bool, _project: Option<String>, branch: Option<String>, _force: bool) -> Result<()> {
    let current_dir = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?;

    // Determine branch to use
    let branch_name = match branch {
        Some(b) => b,
        None => {
            // Get current git branch
            use claude_rag::branch::BranchManager;
            match BranchManager::new(&current_dir) {
                Ok(manager) => manager.current_branch()?,
                Err(_) => "main".to_string(),
            }
        }
    };

    info!("Starting code symbol indexing for branch: {}", branch_name);
    println!("🔍 Indexing code symbols...");
    println!("  Branch: {}", branch_name);
    println!("  Project: {}", current_dir.display());

    // Load config
    let config = match claude_rag::ConfigManager::load(Some(&current_dir)) {
        Ok(config) => config,
        Err(e) => {
            warn!("Failed to load config, using defaults: {}", e);
            eprintln!("Warning: Failed to load config, using defaults");
            claude_rag::Config::default()
        }
    };

    // Open storage
    let storage = claude_rag::storage::sled::StorageManager::open_project_db(&current_dir)?;

    // Load or create HNSW index
    let mut hnsw_index = match storage.load_hnsw()? {
        Some(index) => index,
        None => claude_rag::storage::hnsw::HnswIndex::new(
            config.hnsw.m,
            config.hnsw.ef_construction,
            config.hnsw.ef_search,
        ),
    };

    // Create indexer
    let indexer = claude_rag::indexer::Indexer::from_config(&config)?;
    let api_configured = indexer.is_configured();

    if !api_configured {
        warn!("Embedding API not configured, cannot index code symbols");
        eprintln!("✗ Embedding API not configured");
        eprintln!("  Please configure embedding.api_token in .rag/config.json");
        eprintln!("  Or run in test mode: CLAUDE_RAG_TEST_MODE=1");
        std::process::exit(1);
    }

    // Step 1: Find all code files in the project
    use claude_rag::scanner::FileScanner;
    let scanner = FileScanner::new(&current_dir)?;

    println!("  Scanning for code files...");
    let scanned_files = scanner.scan(true, false, false)?;

    // Filter only source files
    let code_files: Vec<_> = scanned_files
        .into_iter()
        .filter(|f| f.file_type == claude_rag::scanner::FileType::Source)
        .collect();

    println!("  Found {} code files", code_files.len());

    if code_files.is_empty() {
        println!("✓ No code files to index");
        return Ok(());
    }

    // Create progress reporter with configured style
    let reporter = claude_rag::ProgressBarReporter::new(config.progress.style.clone());

    // Step 2 & 3: Extract symbols and index them with branch awareness
    use sha2::{Digest, Sha256};
    use tokio::runtime::Runtime;

    // Check if we're already in a runtime context
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // We're already in an async context, use block_in_place with block_on
        let files_to_index: Vec<(std::path::PathBuf, String)> = code_files
            .iter()
            .map(|f| {
                let relative_path = f.path
                    .strip_prefix(&current_dir)
                    .unwrap_or(&f.path)
                    .to_string_lossy()
                    .to_string();
                let file_id = format!("file:{:x}", Sha256::digest(relative_path.as_bytes()));
                (f.path.clone(), file_id)
            })
            .collect();

        println!("  Extracting and indexing symbols...");

        let (stats, symbols) = tokio::task::block_in_place(|| {
            handle.block_on(async {
                indexer.index_code_files(
                    files_to_index.iter().map(|(p, id)| (p.as_path(), id.as_str())).collect(),
                    &branch_name,
                    &mut hnsw_index,
                ).await
            })
        })?;

        return finish_indexing(stats, symbols, &storage, &hnsw_index, reporter);
    }

    // No runtime context, create new one
    let rt = Runtime::new()?;

    let files_to_index: Vec<(std::path::PathBuf, String)> = code_files
        .iter()
        .map(|f| {
            let relative_path = f.path
                .strip_prefix(&current_dir)
                .unwrap_or(&f.path)
                .to_string_lossy()
                .to_string();
            let file_id = format!("file:{:x}", Sha256::digest(relative_path.as_bytes()));
            (f.path.clone(), file_id)
        })
        .collect();

    println!("  Extracting and indexing symbols...");
    let (stats, symbols) = rt.block_on(async {
        indexer.index_code_files(
            files_to_index.iter().map(|(p, id)| (p.as_path(), id.as_str())).collect(),
            &branch_name,
            &mut hnsw_index,
        ).await
    })?;

    finish_indexing(stats, symbols, &storage, &hnsw_index, reporter)
}

fn finish_indexing(
    stats: claude_rag::indexer::IndexingStats,
    symbols: Vec<claude_rag::models::Symbol>,
    storage: &claude_rag::storage::sled::StorageManager,
    hnsw_index: &claude_rag::storage::hnsw::HnswIndex,
    reporter: claude_rag::ProgressBarReporter,
) -> Result<()> {
    // Store symbols in StorageManager
    println!("  Storing {} symbols...", symbols.len());
    for symbol in &symbols {
        storage.store_symbol_branch(symbol, "main")?;
    }

    reporter.finish();

    // Save HNSW index
    storage.save_hnsw(hnsw_index)?;

    // Print results
    println!("✓ Code symbol indexing complete");
    println!("  Symbols indexed: {}", symbols.len());
    println!("  Embeddings generated: {}", stats.embeddings_generated);

    if stats.errors > 0 {
        println!("  Errors: {}", stats.errors);
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

async fn handle_daemon(daemon_cmd: DaemonCommands) -> Result<()> {
    use claude_rag::daemon::Daemon;
    use claude_rag::daemon::DaemonStatus;
    use std::path::Path;

    let current_dir = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get current directory: {}", e))?;

    // Load config
    let config = match claude_rag::ConfigManager::load(Some(&current_dir)) {
        Ok(config) => config,
        Err(_e) => {
            // Try global config
            match claude_rag::ConfigManager::load(None) {
                Ok(config) => config,
                Err(e) => {
                    error!("Failed to load config: {}", e);
                    eprintln!("✗ Failed to load config: {}", e);
                    eprintln!("  Run 'claude-rag init' to create a configuration file");
                    std::process::exit(1);
                }
            }
        }
    };

    match daemon_cmd {
        DaemonCommands::Start => {
            info!("Starting daemon");
            println!("Starting daemon...");

            // Check if already running
            let daemon = Daemon::new(config.clone());
            match daemon.status().await {
                Ok(DaemonStatus::Running) => {
                    println!("✗ Daemon is already running");
                    println!("  Use 'claude-rag daemon status' to check");
                    std::process::exit(1);
                }
                _ => {}
            }

            // Start daemon
            let mut daemon = Daemon::new(config);
            if let Err(e) = daemon.start().await {
                error!("Failed to start daemon: {}", e);
                eprintln!("✗ Failed to start daemon: {}", e);
                std::process::exit(1);
            }

            println!("✓ Daemon started");
            println!("  Socket: {}", daemon.get_socket_path().display());
            println!("  PID file: {}", daemon.get_pid_file());
        }
        DaemonCommands::Stop => {
            info!("Stopping daemon");
            println!("Stopping daemon...");

            let daemon = Daemon::new(config.clone());
            let status = daemon.status().await?;

            match status {
                DaemonStatus::Running => {
                    // Read PID and send SIGTERM
                    let pid_file = daemon.get_pid_file();
                    if Path::new(pid_file).exists() {
                        let pid_content = fs::read_to_string(pid_file)
                            .map_err(|e| anyhow::anyhow!("Failed to read PID file: {}", e))?;
                        let pid: u32 = pid_content.trim()
                            .parse()
                            .map_err(|_| anyhow::anyhow!("Invalid PID in file"))?;

                        // Send SIGTERM using kill command
                        #[cfg(unix)]
                        {
                            use std::process::Command;
                            let result = Command::new("kill")
                                .arg("-TERM")
                                .arg(pid.to_string())
                                .output();

                            match result {
                                Ok(output) if output.status.success() => {
                                    // Wait a bit and cleanup
                                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                    let _ = fs::remove_file(pid_file);
                                    let socket_path = daemon.get_socket_path();
                                    if socket_path.exists() {
                                        let _ = fs::remove_file(&socket_path);
                                    }
                                    println!("✓ Daemon stopped");
                                }
                                _ => {
                                    error!("Failed to send SIGTERM to daemon");
                                    eprintln!("✗ Failed to stop daemon");
                                    std::process::exit(1);
                                }
                            }
                        }

                        #[cfg(not(unix))]
                        {
                            eprintln!("✗ Daemon stop is not supported on this platform");
                            std::process::exit(1);
                        }
                    } else {
                        eprintln!("✗ PID file not found");
                        std::process::exit(1);
                    }
                }
                DaemonStatus::Stopped => {
                    println!("✗ Daemon is not running");
                    std::process::exit(1);
                }
                DaemonStatus::Unknown => {
                    println!("✗ Daemon status unknown");
                    std::process::exit(1);
                }
            }
        }
        DaemonCommands::Status => {
            debug!("Checking daemon status");
            println!("Daemon status:");

            let daemon = Daemon::new(config);
            let status = daemon.status().await?;

            match status {
                DaemonStatus::Running => {
                    println!("  Status: running");
                    // Try to get PID
                    let pid_file = daemon.get_pid_file();
                    if Path::new(pid_file).exists() {
                        if let Ok(pid_content) = fs::read_to_string(pid_file) {
                            println!("  PID: {}", pid_content.trim());
                        }
                    }
                    println!("  Socket: {}", daemon.get_socket_path().display());
                }
                DaemonStatus::Stopped => {
                    println!("  Status: stopped");
                }
                DaemonStatus::Unknown => {
                    println!("  Status: unknown");
                }
            }
        }
        DaemonCommands::Restart => {
            info!("Restarting daemon");
            println!("Restarting daemon...");

            let daemon = Daemon::new(config.clone());
            let status = daemon.status().await?;

            // Stop if running
            if matches!(status, DaemonStatus::Running) {
                println!("  Stopping existing daemon...");

                // Read PID and send SIGTERM
                let pid_file = daemon.get_pid_file();
                if Path::new(pid_file).exists() {
                    let pid_content = fs::read_to_string(pid_file)
                        .map_err(|e| anyhow::anyhow!("Failed to read PID file: {}", e))?;
                    let pid: u32 = pid_content.trim()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("Invalid PID in file"))?;

                    #[cfg(unix)]
                    {
                        use std::process::Command;
                        let _ = Command::new("kill")
                            .arg("-TERM")
                            .arg(pid.to_string())
                            .output();
                    }

                    // Wait for cleanup
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    let _ = fs::remove_file(pid_file);
                    let socket_path = daemon.get_socket_path();
                    if socket_path.exists() {
                        let _ = fs::remove_file(&socket_path);
                    }
                }
            }

            // Start new daemon
            println!("  Starting new daemon...");
            let mut daemon = Daemon::new(config);
            if let Err(e) = daemon.start().await {
                error!("Failed to start daemon: {}", e);
                eprintln!("✗ Failed to start daemon: {}", e);
                std::process::exit(1);
            }

            println!("✓ Daemon restarted");
            println!("  Socket: {}", daemon.get_socket_path().display());
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
    // Try to use existing runtime, or create a new one if needed
    let result = if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // Use existing runtime
        tokio::task::block_in_place(|| {
            handle.block_on(claude_rag::execute_query_with_time_range(
                query,
                r#type,
                top_k,
                timeline,
                format,
                time_range,
            ))
        })
    } else {
        // Create new runtime
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(claude_rag::execute_query_with_time_range(
            query,
            r#type,
            top_k,
            timeline,
            format,
            time_range,
        ))
    }?;

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

    info!("Skills installed to {}", installer.commands_dir().display());
    println!("✓ Skills installed to: {}", installer.commands_dir().display());
    println!("  Available skills:");
    println!("    /rag:code      - Search source code");
    println!("    /rag:docs      - Search documentation");
    println!("    /rag:query     - Query all indexed content");
    println!("    /rag:session   - Search previous sessions");
    println!("    /rag:timeline  - Build feature timeline");
    Ok(())
}
