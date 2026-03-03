//! Pagination support for large result sets.
//!
//! Provides cursor-based pagination for SQL queries that return
//! more rows than can be efficiently transferred in a single response.

pub mod cursor_store;
