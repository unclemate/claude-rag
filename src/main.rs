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
    Init,
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
        Commands::Init => {
            println!("Initializing knowledge base...");
            // TODO: Implement init
            println!("✓ Knowledge base initialized");
        }
        Commands::Index { all, project, force, r#type } => {
            println!("Indexing...");
            if all {
                println!("  Mode: all projects");
            } else if let Some(p) = project {
                println!("  Project: {}", p);
            }
            if force {
                println!("  Force re-index: yes");
            }
            if let Some(t) = r#type {
                println!("  Type: {}", t);
            }
            // TODO: Implement indexing
            println!("✓ Indexing complete");
        }
        Commands::Daemon { daemon_cmd } => {
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
        }
        Commands::Query { query, r#type, top_k, timeline, format } => {
            println!("Querying: {}", query);
            if let Some(t) = r#type {
                println!("  Type: {}", t);
            }
            println!("  Top-K: {}", top_k);
            println!("  Timeline: {}", timeline);
            println!("  Format: {}", format);
            // TODO: Implement query
            println!("✓ Query complete");
        }
        Commands::Status => {
            println!("Status:");
            // TODO: Implement status
            println!("  Database: initialized");
            println!("  Indexed items: 0");
        }
        Commands::McpServer => {
            println!("Starting MCP server...");
            // TODO: Implement MCP server
            println!("✓ MCP server started");
        }
        Commands::InstallSkills => {
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
        }
    }

    Ok(())
}
