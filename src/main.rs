#[cfg(feature = "vector")]
use sqlite_mcp_rs::adapters::ort_runtime::initialize_ort_dylib_env;
use sqlite_mcp_rs::config::AppConfig;
use sqlite_mcp_rs::server::mcp::SqliteMcpServer;

use rmcp::{ServiceExt, transport::stdio};

fn main() {
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
