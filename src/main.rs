use sqlite_mcp_rs::config::AppConfig;
use sqlite_mcp_rs::server::mcp::SqliteMcpServer;

use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() {
    match AppConfig::from_env() {
        Ok(config) => {
            init_tracing(config.log_level.as_str());
            tracing::info!(
                persist_enabled = config.persist_root.is_some(),
                "starting MCP stdio server"
            );
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
