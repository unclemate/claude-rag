//! 日志系统初始化和配置模块
//!
//! 提供基于 tracing-subscriber 的结构化日志系统，支持：
//! - 控制台输出（人类可读格式）
//! - 文件输出（JSON 格式）
//! - 每日日志轮转
//! - 非阻塞写入
//! - RUST_LOG 环境变量控制

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};
use std::io;

/// 日志级别配置
#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    /// 最详细的级别，用于追踪执行流程
    Trace,
    /// 调试信息，用于开发诊断
    Debug,
    /// 一般信息，用于记录正常运行状态
    Info,
    /// 警告信息，用于标记潜在问题
    Warn,
    /// 错误信息，用于记录错误和异常
    Error,
}

impl LogLevel {
    /// 获取日志级别的字符串表示
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// 日志配置选项（用于运行时配置）
#[derive(Debug, Clone)]
pub struct LoggingOptions {
    /// 默认日志级别
    pub default_level: LogLevel,
    /// 是否启用文件日志
    pub enable_file_logging: bool,
    /// 自定义日志目录（None 表示使用默认 .rag/logs/）
    pub log_dir: Option<std::path::PathBuf>,
    /// 文件日志是否使用 JSON 格式
    pub json_format: bool,
    /// 是否包含 span 事件（用于追踪异步调用链）
    pub include_spans: bool,
    /// 是否启用每日轮转
    pub daily_rotation: bool,
}

impl Default for LoggingOptions {
    fn default() -> Self {
        Self {
            default_level: LogLevel::Info,
            enable_file_logging: true,
            log_dir: None,
            json_format: true,
            include_spans: false,
            daily_rotation: true,
        }
    }
}

// ====== 辅助函数 ======

/// 构建环境过滤器，支持 RUST_LOG 环境变量
///
/// 使用常量避免运行时解析错误，提高编译时安全性。
fn build_env_filter(default_level: LogLevel) -> EnvFilter {
    const DIRECTIVES: &[&str] = &[
        "sled=info",
        "notify=warn",
        "tokio=warn",
        "hyper=warn",
        "reqwest=warn",
    ];

    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("claude_rag={}", default_level.as_str())))
        .add_directive(DIRECTIVES[0].parse().expect("invalid directive: sled=info"))
        .add_directive(DIRECTIVES[1].parse().expect("invalid directive: notify=warn"))
        .add_directive(DIRECTIVES[2].parse().expect("invalid directive: tokio=warn"))
        .add_directive(DIRECTIVES[3].parse().expect("invalid directive: hyper=warn"))
        .add_directive(DIRECTIVES[4].parse().expect("invalid directive: reqwest=warn"))
}

/// 解析日志目录路径
///
/// 避免不必要的 clone，使用 as_deref() 引用。
fn resolve_log_dir(
    log_dir: &Option<std::path::PathBuf>,
    project_path: &std::path::Path,
) -> std::path::PathBuf {
    log_dir
        .as_deref()
        .map(|p| project_path.join(p))
        .unwrap_or_else(|| project_path.join(".rag/logs"))
}

// ====== 公开 API ======

/// 初始化日志系统
///
/// # 参数
/// - `project_path`: 项目根目录路径
/// - `config`: 日志配置选项
///
/// # 返回
/// 返回 `WorkerGuard`，**必须在 main 函数中保持存活**，否则日志会停止写入。
///
/// # 示例
/// ```no_run
/// use claude_rag::logging::init_logging_default;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let project_path = std::env::current_dir()?;
///     let _log_guard = init_logging_default(&project_path)?;
///     // ... 程序代码 ...
///     Ok(())
/// }
/// ```
pub fn init_logging(
    project_path: &std::path::Path,
    config: LoggingOptions,
) -> Result<WorkerGuard, anyhow::Error> {
    let env_filter = build_env_filter(config.default_level);

    // 控制台层（人类可读格式）
    let console_layer = fmt::layer()
        .with_writer(io::stderr)
        .with_target(false)
        .with_span_events(if config.include_spans {
            FmtSpan::NEW | FmtSpan::CLOSE
        } else {
            FmtSpan::NONE
        })
        .with_filter(env_filter.clone());

    if config.enable_file_logging {
        let log_dir = resolve_log_dir(&config.log_dir, project_path);

        std::fs::create_dir_all(&log_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create log directory {}: {}", log_dir.display(), e))?;

        let file_appender = if config.daily_rotation {
            tracing_appender::rolling::daily(&log_dir, "claude-rag")
        } else {
            tracing_appender::rolling::never(&log_dir, "claude-rag")
        };

        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        if config.json_format {
            tracing_subscriber::registry()
                .with(console_layer)
                .with(fmt::layer().json().with_writer(non_blocking).with_filter(env_filter.clone()))
                .init();
        } else {
            tracing_subscriber::registry()
                .with(console_layer)
                .with(fmt::layer().with_writer(non_blocking).with_filter(env_filter.clone()))
                .init();
        }

        Ok(guard)
    } else {
        tracing_subscriber::registry()
            .with(console_layer)
            .init();
        Ok(create_dummy_guard())
    }
}

/// 创建一个 dummy guard，当不需要文件日志时使用
///
/// # 公开导出
/// 此函数在日志初始化失败时可用作备用 guard，确保类型一致。
pub fn create_dummy_guard() -> WorkerGuard {
    use std::io::sink;
    let (non_blocking, guard) = tracing_appender::non_blocking(sink());
    drop(non_blocking);
    guard
}

/// 简化版初始化，使用默认配置
///
/// # 参数
/// - `project_path`: 项目根目录路径
///
/// # 返回
/// 返回 `WorkerGuard`，**必须在 main 函数中保持存活**。
pub fn init_logging_default(project_path: &std::path::Path) -> Result<WorkerGuard, anyhow::Error> {
    init_logging(project_path, LoggingOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_as_str() {
        assert_eq!(LogLevel::Trace.as_str(), "trace");
        assert_eq!(LogLevel::Debug.as_str(), "debug");
        assert_eq!(LogLevel::Info.as_str(), "info");
        assert_eq!(LogLevel::Warn.as_str(), "warn");
        assert_eq!(LogLevel::Error.as_str(), "error");
    }

    #[test]
    fn test_logging_options_default() {
        let config = LoggingOptions::default();
        assert!(matches!(config.default_level, LogLevel::Info));
        assert!(config.enable_file_logging);
        assert!(config.json_format);
        assert!(config.daily_rotation);
        assert!(!config.include_spans);
    }
}
