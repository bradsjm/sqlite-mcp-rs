//! Request and response types for MCP tool contracts.
//!
//! This module defines the data structures used in MCP tool requests and responses,
//! organized by functional area (database, SQL, vector, queue, import).

pub mod common;
pub mod db;
pub mod import;
pub mod queue;
pub mod schema;
pub mod sql;

#[cfg(feature = "vector")]
pub mod vector;
