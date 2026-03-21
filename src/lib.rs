//! SQLite MCP Server
//!
//! A Model Context Protocol (MCP) server that provides SQLite database operations
//! with support for vector search, job queues, and data import capabilities.
//!
//! # Features
//!
//! - **Database Management**: Open, close, and list SQLite databases (memory or persisted)
//! - **SQL Operations**: Execute queries, statements, and batches with policy enforcement
//! - **Vector Search**: Create collections, upsert documents, and perform similarity search
//!   (requires `vector` feature)
//! - **Job Queue**: Push and wait for jobs in database-backed queues
//! - **Data Import**: Import CSV and JSON data into tables
//!
//! # Architecture
//!
//! The crate is organized into modules:
//! - `config`: Application configuration from environment variables
//! - `contracts`: Request/response types for MCP tool calls
//! - `db`: Database registry and persistence management
//! - `errors`: Error types and protocol error mapping
//! - `pagination`: Cursor-based pagination for large result sets
//! - `policy`: SQL validation and security policies
//! - `server`: MCP server implementation and tool handlers
//! - `tools`: Business logic for each MCP tool
//! - `adapters`: External service integrations (embeddings, reranking)

pub mod config;
pub mod contracts;
pub mod db;
pub mod errors;
pub mod pagination;
pub mod policy;
pub mod server;
pub mod tools;

#[cfg(feature = "local-embeddings")]
pub mod adapters;

/// Default database identifier used when none is specified.
pub const DEFAULT_DB_ID: &str = "default";
