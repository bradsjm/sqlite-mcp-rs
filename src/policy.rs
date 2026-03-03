//! SQL policy validation and security scanning.
//!
//! This module provides SQL statement validation, security scanning, and policy
//! enforcement to prevent dangerous operations and enforce resource limits.

use crate::contracts::sql::BatchStatement;
use crate::errors::{AppError, AppResult};

/// SQL policy configuration for validation and resource limits.
///
/// Defines constraints on SQL operations including statement length,
/// result size limits, and database size limits.
#[derive(Debug, Clone)]
pub struct SqlPolicy {
    /// Maximum allowed SQL statement length in characters.
    pub max_sql_length: usize,
    /// Maximum number of statements allowed in a batch.
    pub max_statements: usize,
    /// Maximum number of rows to return per query.
    pub max_rows: usize,
    /// Maximum response size in bytes.
    pub max_bytes: usize,
    /// Maximum database file size in bytes.
    pub max_db_bytes: u64,
}

impl SqlPolicy {
    /// Validates that SQL statement length is within configured limits.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::LimitExceeded`] if the SQL exceeds `max_sql_length`.
    pub fn validate_sql_length(&self, sql: &str) -> AppResult<()> {
        if sql.len() > self.max_sql_length {
            return Err(AppError::LimitExceeded(format!(
                "sql length exceeds {} characters",
                self.max_sql_length
            )));
        }
        Ok(())
    }
}

/// Splits a SQL string into individual statements.
///
/// Handles quoted strings, comments, and semicolons correctly.
/// Returns a vector of trimmed SQL statements.
pub fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut state = ScanState::Normal;
    let mut start = 0usize;
    let mut index = 0usize;
    let mut statements = Vec::new();
    let bytes = sql.as_bytes();

    while index < bytes.len() {
        let byte = bytes[index];
        match state {
            ScanState::Normal => {
                if byte == b'\'' {
                    state = ScanState::SingleQuote;
                } else if byte == b'"' {
                    state = ScanState::DoubleQuote;
                } else if byte == b'[' {
                    state = ScanState::BracketQuote;
                } else if byte == b'-' && bytes.get(index + 1) == Some(&b'-') {
                    state = ScanState::LineComment;
                    index += 1;
                } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    state = ScanState::BlockComment;
                    index += 1;
                } else if byte == b';' {
                    let segment = sql[start..index].trim();
                    if !segment.is_empty() {
                        statements.push(segment.to_string());
                    }
                    start = index + 1;
                }
            }
            ScanState::SingleQuote => {
                if byte == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        index += 1;
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::DoubleQuote => {
                if byte == b'"' {
                    if bytes.get(index + 1) == Some(&b'"') {
                        index += 1;
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::BracketQuote => {
                if byte == b']' {
                    state = ScanState::Normal;
                }
            }
            ScanState::LineComment => {
                if byte == b'\n' {
                    state = ScanState::Normal;
                }
            }
            ScanState::BlockComment => {
                if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = ScanState::Normal;
                    index += 1;
                }
            }
        }

        index += 1;
    }

    let trailing = sql[start..].trim();
    if !trailing.is_empty() {
        statements.push(trailing.to_string());
    }

    statements
}

/// Checks if SQL contains blocked statements (ATTACH, LOAD_EXTENSION).
///
/// Returns true if the SQL contains any blocked statements that could
/// compromise security or access unauthorized resources.
pub fn contains_blocked_sql(sql: &str) -> bool {
    split_sql_statements(sql).iter().any(|statement| {
        let normalized = normalize_sql_outside_literals(statement);
        normalized.trim_start().starts_with("ATTACH") || contains_load_extension_call(&normalized)
    })
}

/// Checks if SQL references a protected table name.
///
/// Protected tables (like `_vector_collections`) should not be modified
/// directly through SQL to maintain data integrity.
pub fn contains_protected_table_reference(sql: &str, table: &str) -> bool {
    let normalized = normalize_sql_outside_literals(sql);
    contains_identifier_token(&normalized, &table.to_ascii_uppercase())
}

/// Checks if a batch of statements appears destructive.
///
/// Destructive operations include DROP, TRUNCATE, and DELETE without WHERE clauses.
/// These require explicit confirmation via `confirm_destructive` flag.
pub fn looks_destructive_batch(statements: &[BatchStatement]) -> bool {
    statements.iter().any(|statement| {
        let normalized = statement.sql.trim().to_ascii_uppercase();
        if normalized.starts_with("DROP ") || normalized.starts_with("TRUNCATE ") {
            return true;
        }

        normalized.starts_with("DELETE FROM ") && !normalized.contains(" WHERE ")
    })
}

/// Validates that a string is a valid SQL identifier.
///
/// Valid identifiers start with a letter or underscore and contain only
/// alphanumeric characters and underscores. They must match the pattern
/// `^[A-Za-z_][A-Za-z0-9_]*$`.
pub fn is_valid_identifier(identifier: &str) -> bool {
    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }

    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Normal,
    SingleQuote,
    DoubleQuote,
    BracketQuote,
    LineComment,
    BlockComment,
}

fn normalize_sql_outside_literals(sql: &str) -> String {
    let mut normalized = String::with_capacity(sql.len());
    let mut state = ScanState::Normal;
    let bytes = sql.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        let byte = bytes[index];

        match state {
            ScanState::Normal => {
                if byte == b'\'' {
                    normalized.push(' ');
                    state = ScanState::SingleQuote;
                } else if byte == b'"' {
                    normalized.push(' ');
                    state = ScanState::DoubleQuote;
                } else if byte == b'[' {
                    normalized.push(' ');
                    state = ScanState::BracketQuote;
                } else if byte == b'-' && bytes.get(index + 1) == Some(&b'-') {
                    normalized.push(' ');
                    normalized.push(' ');
                    state = ScanState::LineComment;
                    index += 1;
                } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    normalized.push(' ');
                    normalized.push(' ');
                    state = ScanState::BlockComment;
                    index += 1;
                } else {
                    normalized.push((byte as char).to_ascii_uppercase());
                }
            }
            ScanState::SingleQuote => {
                normalized.push(' ');
                if byte == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        normalized.push(' ');
                        index += 1;
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::DoubleQuote => {
                normalized.push(' ');
                if byte == b'"' {
                    if bytes.get(index + 1) == Some(&b'"') {
                        normalized.push(' ');
                        index += 1;
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::BracketQuote => {
                normalized.push(' ');
                if byte == b']' {
                    state = ScanState::Normal;
                }
            }
            ScanState::LineComment => {
                if byte == b'\n' {
                    normalized.push('\n');
                    state = ScanState::Normal;
                } else {
                    normalized.push(' ');
                }
            }
            ScanState::BlockComment => {
                normalized.push(' ');
                if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    normalized.push(' ');
                    state = ScanState::Normal;
                    index += 1;
                }
            }
        }

        index += 1;
    }

    normalized
}

fn contains_load_extension_call(sql: &str) -> bool {
    const TOKEN: &str = "LOAD_EXTENSION";
    let bytes = sql.as_bytes();
    let mut index = 0usize;

    while index + TOKEN.len() <= bytes.len() {
        if &sql[index..index + TOKEN.len()] == TOKEN {
            let before_ok = if index == 0 {
                true
            } else {
                !is_identifier_char(bytes[index - 1] as char)
            };

            let after = index + TOKEN.len();
            let after_ok = if after >= bytes.len() {
                true
            } else {
                !is_identifier_char(bytes[after] as char)
            };

            if before_ok && after_ok {
                let mut next = after;
                while next < bytes.len() && (bytes[next] as char).is_ascii_whitespace() {
                    next += 1;
                }
                if next < bytes.len() && bytes[next] == b'(' {
                    return true;
                }
            }
        }

        index += 1;
    }

    false
}

fn contains_identifier_token(sql: &str, token: &str) -> bool {
    let bytes = sql.as_bytes();
    let token_bytes = token.as_bytes();
    let mut index = 0usize;

    while index + token_bytes.len() <= bytes.len() {
        if &bytes[index..index + token_bytes.len()] == token_bytes {
            let before_ok = if index == 0 {
                true
            } else {
                !is_identifier_char(bytes[index - 1] as char)
            };

            let after = index + token_bytes.len();
            let after_ok = if after >= bytes.len() {
                true
            } else {
                !is_identifier_char(bytes[after] as char)
            };

            if before_ok && after_ok {
                return true;
            }
        }

        index += 1;
    }

    false
}

fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use crate::contracts::sql::BatchStatement;

    use super::{
        contains_blocked_sql, contains_protected_table_reference, is_valid_identifier,
        looks_destructive_batch, split_sql_statements,
    };

    #[test]
    fn blocks_attach_and_load_extension() {
        assert!(contains_blocked_sql("attach database 'x.db' as x"));
        assert!(contains_blocked_sql("select load_extension('bad')"));
        assert!(!contains_blocked_sql("select 'ATTACH', 'LOAD_EXTENSION('"));
        assert!(!contains_blocked_sql("select 1"));
    }

    #[test]
    fn detects_destructive_batch() {
        let statements = vec![BatchStatement {
            sql: "delete from users".to_string(),
            params: None,
        }];
        assert!(looks_destructive_batch(&statements));
    }

    #[test]
    fn splits_statements() {
        let parts = split_sql_statements("select 1; select 2;");
        assert_eq!(parts.len(), 2);

        let quoted = split_sql_statements("select 'a;b'; select 2");
        assert_eq!(quoted.len(), 2);

        let commented = split_sql_statements("select 1; -- ;\n select 2;");
        assert_eq!(commented.len(), 2);
    }

    #[test]
    fn validates_identifiers() {
        assert!(is_valid_identifier("users"));
        assert!(is_valid_identifier("_internal_1"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("1users"));
        assert!(!is_valid_identifier("drop table"));
    }

    #[test]
    fn detects_protected_table_reference_outside_literals() {
        assert!(contains_protected_table_reference(
            "update _vector_collections set last_updated = current_timestamp",
            "_vector_collections"
        ));
        assert!(!contains_protected_table_reference(
            "select '_vector_collections'",
            "_vector_collections"
        ));
    }
}
