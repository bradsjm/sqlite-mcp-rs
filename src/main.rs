#[cfg(feature = "vector")]
use sqlite_mcp_rs::adapters::ort_runtime::initialize_ort_dylib_env;
use sqlite_mcp_rs::config::AppConfig;
use sqlite_mcp_rs::server::mcp::SqliteMcpServer;
use std::io::{self, Write};

use rmcp::{ServiceExt, transport::stdio};

fn main() {
    if should_print_help(std::env::args().skip(1)) {
        if let Err(error) = print_help_output() {
            eprintln!("failed to write help output: {error}");
            std::process::exit(1);
        }
        return;
    }

    match AppConfig::from_env() {
        Ok(config) => {
            init_tracing(config.log_level.as_str());
            #[cfg(feature = "vector")]
            if let Err(error) = initialize_ort_dylib_env(config.embedding.cache_dir.clone()) {
                tracing::error!(error = %error, "failed to initialize ORT runtime");
                std::process::exit(1);
            }

            tracing::info!(
                persist_enabled = config.persist_root.is_some(),
                "starting MCP stdio server"
            );
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("failed to create tokio runtime: {error}");
                    std::process::exit(1);
                }
            };

            runtime.block_on(async move {
                let server = SqliteMcpServer::new(config);
                match server.serve(stdio()).await {
                    Ok(service) => {
                        if let Err(error) = service.waiting().await {
                            tracing::error!(error = %error, "server shutdown with error");
                            std::process::exit(1);
                        }
                    }
                    Err(error) => {
                        tracing::error!(error = %error, "failed to start MCP stdio service");
                        std::process::exit(1);
                    }
                }
            });
        }
        Err(error) => {
            eprintln!("invalid startup configuration: {error}");
            std::process::exit(1);
        }
    }
}

fn init_tracing(filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .init();
}

fn should_print_help<I>(args: I) -> bool
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    args.into_iter().any(|arg| {
        let arg = arg.as_ref();
        arg == "--help" || arg == "-h"
    })
}

fn print_help_output() -> io::Result<()> {
    let output = build_help_output();
    let mut stdout = io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()
}

fn build_help_output() -> String {
    let mut out = String::new();

    out.push_str("sqlite-mcp-rs\n");
    out.push_str("SQLite MCP server over stdio\n\n");

    out.push_str("Usage:\n");
    out.push_str("  sqlite-mcp-rs\n");
    out.push_str("  sqlite-mcp-rs --help\n\n");

    out.push_str("Environment variables\n");
    out.push_str("  SQLITE_PERSIST_ROOT (optional)\n");
    out.push_str("  SQLITE_LOG_LEVEL=info\n");
    out.push_str("  SQLITE_MAX_SQL_LENGTH=20000\n");
    out.push_str("  SQLITE_MAX_STATEMENTS=50\n");
    out.push_str("  SQLITE_MAX_ROWS=500\n");
    out.push_str("  SQLITE_MAX_BYTES=1048576\n");
    out.push_str("  SQLITE_MAX_DB_BYTES=100000000\n");
    out.push_str("  SQLITE_MAX_PERSISTED_LIST_ENTRIES=500\n");
    out.push_str("  SQLITE_CURSOR_TTL_SECONDS=600\n");
    out.push_str("  SQLITE_CURSOR_CAPACITY=500\n");

    #[cfg(feature = "vector")]
    {
        out.push_str("\nVector feature environment variables\n");
        out.push_str("  SQLITE_MAX_VECTOR_TOP_K=200\n");
        out.push_str("  SQLITE_MAX_RERANK_FETCH_K=500\n");
        out.push_str("  SQLITE_EMBEDDING_PROVIDER=fastembed\n");
        out.push_str("  SQLITE_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5\n");
        out.push_str("  SQLITE_EMBEDDING_CACHE_DIR (optional)\n");
        out.push_str("  SQLITE_RERANKER_PROVIDER (optional, default when enabled: fastembed)\n");
        out.push_str(
            "  SQLITE_RERANKER_MODEL (optional, default when enabled: BAAI/bge-reranker-base)\n",
        );
        out.push_str("  SQLITE_RERANKER_CACHE_DIR (optional)\n");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{build_help_output, should_print_help};

    #[test]
    fn detects_short_and_long_help_flags() {
        assert!(should_print_help(["-h"]));
        assert!(should_print_help(["--help"]));
        assert!(should_print_help(["--verbose", "-h"]));
        assert!(!should_print_help(["--verbose"]));
    }

    #[test]
    fn help_output_lists_usage_and_core_environment_variables() {
        let help = build_help_output();

        assert!(help.contains("sqlite-mcp-rs"));
        assert!(help.contains("Usage:"));
        assert!(help.contains("sqlite-mcp-rs --help"));
        assert!(help.contains("SQLITE_LOG_LEVEL=info"));
        assert!(help.contains("SQLITE_MAX_ROWS=500"));
        assert!(help.contains("SQLITE_CURSOR_CAPACITY=500"));
    }
}
