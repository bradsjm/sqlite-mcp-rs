use clap::{Parser, ValueEnum};
#[cfg(feature = "vector")]
use sqlite_mcp_rs::adapters::ort_runtime::initialize_ort_dylib_env;
use sqlite_mcp_rs::config::AppConfig;
use sqlite_mcp_rs::server::mcp::SqliteMcpServer;
use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServiceExt, transport::stdio};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Transport {
    Stdio,
    Http,
}

#[derive(Debug, Parser)]
#[command(
    name = "sqlite-mcp-rs",
    about = "SQLite MCP server with stdio and optional HTTP transport"
)]
struct Cli {
    #[arg(long, value_enum, default_value_t = Transport::Stdio)]
    transport: Transport,
    #[arg(long, default_value = "localhost")]
    host: String,
    #[arg(long, default_value_t = 3000)]
    port: u16,
}

fn main() {
    let cli = Cli::parse();

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
                transport = ?cli.transport,
                "starting MCP server"
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
                if let Err(error) = run_server(config, cli).await {
                    tracing::error!(error = %error, "server shutdown with error");
                    std::process::exit(1);
                }
            });
        }
        Err(error) => {
            eprintln!("invalid startup configuration: {error}");
            std::process::exit(1);
        }
    }
}

async fn run_server(config: AppConfig, cli: Cli) -> Result<(), String> {
    match cli.transport {
        Transport::Stdio => run_stdio_server(config).await,
        Transport::Http => run_http_server(config, cli.host, cli.port).await,
    }
}

async fn run_stdio_server(config: AppConfig) -> Result<(), String> {
    tracing::info!("building MCP server for stdio transport");
    let server = SqliteMcpServer::new(config)
        .map_err(|error| format!("failed to initialize MCP server: {error}"))?;
    tracing::info!("MCP stdio server initialization complete");
    let service = server
        .serve(stdio())
        .await
        .map_err(|error| format!("failed to start MCP stdio service: {error}"))?;
    let _ = service
        .waiting()
        .await
        .map_err(|error| format!("stdio server shutdown with error: {error}"))?;
    Ok(())
}

async fn run_http_server(config: AppConfig, host: String, port: u16) -> Result<(), String> {
    tracing::info!("building MCP server for HTTP transport");
    let server = SqliteMcpServer::new(config)
        .map_err(|error| format!("failed to initialize MCP server: {error}"))?;
    tracing::info!("MCP HTTP server initialization complete");
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let app = Router::new().nest_service("/mcp", service);
    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(bind_addr.as_str())
        .await
        .map_err(|error| format!("failed to bind HTTP listener on {bind_addr}: {error}"))?;

    tracing::info!(endpoint = %format!("http://{bind_addr}/mcp"), "starting MCP HTTP server");

    axum::serve(listener, app)
        .await
        .map_err(|error| format!("http server shutdown with error: {error}"))
}

fn init_tracing(filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .init();
}

#[cfg(test)]
mod tests {
    use super::{Cli, Transport};
    use clap::Parser;

    #[test]
    fn cli_defaults_to_stdio_localhost_and_port_3000() {
        let cli = Cli::parse_from(["sqlite-mcp-rs"]);
        assert!(matches!(cli.transport, Transport::Stdio));
        assert_eq!(cli.host, "localhost");
        assert_eq!(cli.port, 3000);
    }

    #[test]
    fn cli_accepts_http_host_and_port() {
        let cli = Cli::parse_from([
            "sqlite-mcp-rs",
            "--transport",
            "http",
            "--host",
            "127.0.0.1",
            "--port",
            "8123",
        ]);

        assert!(matches!(cli.transport, Transport::Http));
        assert_eq!(cli.host, "127.0.0.1");
        assert_eq!(cli.port, 8123);
    }

    #[test]
    fn cli_rejects_invalid_transport() {
        let result = Cli::try_parse_from(["sqlite-mcp-rs", "--transport", "sse"]);
        assert!(result.is_err());
    }
}
