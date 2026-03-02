use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use rusqlite::Statement;
use rusqlite::types::{Value as SqlValue, ValueRef};
use serde_json::{Map, Value, json};

use crate::DEFAULT_DB_ID;
use crate::contracts::common::{ToolEnvelope, ToolHint};
use crate::contracts::sql::{
    BatchResultKind, SqlBatchData, SqlBatchRequest, SqlBatchResult, SqlExecuteData,
    SqlExecuteRequest, SqlParams, SqlQueryData, SqlQueryRequest,
};
use crate::db::persistence::enforce_db_size_limit;
use crate::db::registry::DbRegistry;
use crate::errors::{AppError, AppResult};
use crate::pagination::cursor_store::{CursorState, CursorStore};
use crate::policy::{
    SqlPolicy, contains_blocked_sql, looks_destructive_batch, split_sql_statements,
};
use crate::server::finalize::finalize_tool;

pub fn sql_query(
    registry: &DbRegistry,
    cursor_store: &mut CursorStore,
    policy: &SqlPolicy,
    request: SqlQueryRequest,
) -> AppResult<ToolEnvelope<SqlQueryData>> {
    let started = Instant::now();

    let QueryPlan {
        db_id,
        sql,
        params,
        max_rows,
        max_bytes,
        existing_cursor,
        existing_cursor_id,
    } = resolve_query_request(cursor_store, policy, request)?;

    policy.validate_sql_length(&sql)?;
    if contains_blocked_sql(&sql) {
        return Err(AppError::InvalidInput(
            "sql contains blocked statements".to_string(),
        ));
    }

    if split_sql_statements(&sql).len() != 1 {
        return Err(AppError::InvalidInput(
            "sql_query requires exactly one SQL statement".to_string(),
        ));
    }

    let connection = registry.get_connection(Some(&db_id))?;
    let mut statement = connection.prepare(&sql)?;

    if !statement.readonly() {
        return Err(AppError::InvalidInput(
            "sql_query only accepts read-only statements".to_string(),
        ));
    }

    bind_params(&mut statement, params.as_ref())?;
    let columns = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();

    let offset = existing_cursor.as_ref().map_or(0, |cursor| cursor.offset);
    let mut rows = statement.raw_query();
    let mut consumed = 0usize;
    let mut returned_rows = Vec::new();
    let mut used_bytes = 0usize;
    let mut truncated = false;

    while let Some(row) = rows.next()? {
        if consumed < offset {
            consumed += 1;
            continue;
        }

        if returned_rows.len() >= max_rows {
            truncated = true;
            break;
        }

        let row_map = row_to_json_map(row)?;
        let row_size = serde_json::to_vec(&row_map)
            .map_err(|error| AppError::Dependency(format!("failed to encode query row: {error}")))?
            .len();

        if used_bytes + row_size > max_bytes {
            if returned_rows.is_empty() {
                return Err(AppError::LimitExceeded(
                    "a single result row exceeds max_bytes".to_string(),
                ));
            }
            truncated = true;
            break;
        }

        used_bytes += row_size;
        returned_rows.push(row_map);
        consumed += 1;
    }

    let next_offset = offset + returned_rows.len();
    let mut hints = Vec::new();
    let mut next_cursor = None;

    if truncated {
        let state = CursorState {
            db_id: db_id.clone(),
            fingerprint: fingerprint_query(&db_id, &sql, params.as_ref(), max_rows, max_bytes)?,
            offset: next_offset,
            sql: sql.clone(),
            params: params.clone(),
            max_rows,
            max_bytes,
        };
        if let Some(cursor_id) = existing_cursor_id {
            cursor_store.delete(&cursor_id);
        }
        let cursor_id = cursor_store.create(state);

        hints.push(ToolHint {
            tool: "sql_query".to_string(),
            arguments: json!({ "db_id": db_id, "cursor": cursor_id }),
            reason: "Continue reading the remaining rows with this cursor.".to_string(),
        });
        next_cursor = Some(cursor_id);
    } else if let Some(cursor_id) = existing_cursor_id {
        cursor_store.delete(&cursor_id);
    }

    let data = SqlQueryData {
        columns,
        row_count: returned_rows.len(),
        rows: returned_rows,
        truncated,
        next_cursor: next_cursor.clone(),
    };

    Ok(finalize_tool(
        "Query executed.",
        data,
        started,
        hints,
        Some(truncated),
        next_cursor,
    ))
}

pub fn sql_execute(
    registry: &DbRegistry,
    policy: &SqlPolicy,
    request: SqlExecuteRequest,
) -> AppResult<ToolEnvelope<SqlExecuteData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());

    policy.validate_sql_length(&request.sql)?;
    if contains_blocked_sql(&request.sql) {
        return Err(AppError::InvalidInput(
            "sql contains blocked statements".to_string(),
        ));
    }
    if split_sql_statements(&request.sql).len() != 1 {
        return Err(AppError::InvalidInput(
            "sql_execute requires exactly one SQL statement".to_string(),
        ));
    }

    let connection = registry.get_connection(Some(&db_id))?;
    let persisted_path = registry.persisted_path(Some(&db_id))?;
    let mut statement = connection.prepare(&request.sql)?;
    if statement.readonly() {
        return Err(AppError::InvalidInput(
            "sql_execute only accepts non-read statements".to_string(),
        ));
    }

    bind_params(&mut statement, request.params.as_ref())?;
    let rows_affected = statement.raw_execute()?;
    let last_insert_rowid = if request
        .sql
        .trim_start()
        .to_ascii_uppercase()
        .starts_with("INSERT")
    {
        Some(connection.last_insert_rowid())
    } else {
        None
    };
    enforce_db_size_limit(persisted_path.as_deref(), policy.max_db_bytes)?;

    Ok(finalize_tool(
        "Statement executed.",
        SqlExecuteData {
            rows_affected: rows_affected as u64,
            last_insert_rowid,
        },
        started,
        Vec::new(),
        None,
        None,
    ))
}

pub fn sql_batch(
    registry: &DbRegistry,
    policy: &SqlPolicy,
    request: SqlBatchRequest,
) -> AppResult<ToolEnvelope<SqlBatchData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());

    if request.statements.is_empty() {
        return Err(AppError::InvalidInput(
            "sql_batch requires at least one statement".to_string(),
        ));
    }
    if request.statements.len() > policy.max_statements {
        return Err(AppError::LimitExceeded(format!(
            "sql_batch exceeded the maximum of {} statements",
            policy.max_statements
        )));
    }

    if looks_destructive_batch(&request.statements) && !request.confirm_destructive {
        return Err(AppError::PreconditionRequired(
            "destructive batch requires confirm_destructive=true".to_string(),
        ));
    }

    let connection = registry.get_connection(Some(&db_id))?;
    let persisted_path = registry.persisted_path(Some(&db_id))?;
    if request.transaction == crate::contracts::sql::BatchTransactionMode::Required {
        connection.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
    }

    let mut results = Vec::with_capacity(request.statements.len());
    for (index, statement_request) in request.statements.iter().enumerate() {
        policy.validate_sql_length(&statement_request.sql)?;
        if statement_request.sql.trim().is_empty() {
            rollback_if_needed(connection, request.transaction)?;
            return Err(AppError::InvalidInput(format!(
                "statement {index} is empty"
            )));
        }
        if contains_blocked_sql(&statement_request.sql) {
            rollback_if_needed(connection, request.transaction)?;
            return Err(AppError::InvalidInput(format!(
                "statement {index} contains blocked SQL"
            )));
        }
        if split_sql_statements(&statement_request.sql).len() != 1 {
            rollback_if_needed(connection, request.transaction)?;
            return Err(AppError::InvalidInput(format!(
                "statement {index} must contain exactly one SQL statement"
            )));
        }

        let mut statement = match connection.prepare(&statement_request.sql) {
            Ok(statement) => statement,
            Err(error) => {
                rollback_if_needed(connection, request.transaction)?;
                return Err(error.into());
            }
        };

        if let Err(error) = bind_params(&mut statement, statement_request.params.as_ref()) {
            rollback_if_needed(connection, request.transaction)?;
            return Err(error);
        }

        if statement.readonly() {
            let mut row_iter = statement.raw_query();
            while row_iter.next()?.is_some() {}

            results.push(SqlBatchResult {
                index,
                kind: BatchResultKind::Query,
                rows_affected: 0,
                last_insert_rowid: None,
            });
        } else {
            let rows_affected = match statement.raw_execute() {
                Ok(affected) => affected,
                Err(error) => {
                    rollback_if_needed(connection, request.transaction)?;
                    return Err(error.into());
                }
            };

            let last_insert_rowid = if statement_request
                .sql
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("INSERT")
            {
                Some(connection.last_insert_rowid())
            } else {
                None
            };

            results.push(SqlBatchResult {
                index,
                kind: BatchResultKind::Execute,
                rows_affected: rows_affected as u64,
                last_insert_rowid,
            });

            if let Err(error) =
                enforce_db_size_limit(persisted_path.as_deref(), policy.max_db_bytes)
            {
                rollback_if_needed(connection, request.transaction)?;
                return Err(error);
            }
        }
    }

    if request.transaction == crate::contracts::sql::BatchTransactionMode::Required {
        connection.execute_batch("COMMIT")?;
    }

    Ok(finalize_tool(
        "Batch executed.",
        SqlBatchData {
            transaction: request.transaction,
            executed: results.len(),
            results,
        },
        started,
        Vec::new(),
        None,
        None,
    ))
}

fn rollback_if_needed(
    connection: &rusqlite::Connection,
    mode: crate::contracts::sql::BatchTransactionMode,
) -> AppResult<()> {
    if mode == crate::contracts::sql::BatchTransactionMode::Required {
        connection.execute_batch("ROLLBACK")?;
    }
    Ok(())
}

fn resolve_query_request(
    cursor_store: &mut CursorStore,
    policy: &SqlPolicy,
    request: SqlQueryRequest,
) -> AppResult<QueryPlan> {
    if let Some(cursor) = request.cursor.clone() {
        if request.sql.is_some() || request.params.is_some() {
            return Err(AppError::InvalidInput(
                "cursor must not be combined with sql or params".to_string(),
            ));
        }

        let Some(state) = cursor_store.get(&cursor) else {
            return Err(AppError::NotFound(
                "cursor not found or expired; restart the query".to_string(),
            ));
        };

        let fingerprint = fingerprint_query(
            &state.db_id,
            &state.sql,
            state.params.as_ref(),
            state.max_rows,
            state.max_bytes,
        )?;
        if state.fingerprint != fingerprint {
            return Err(AppError::NotFound(
                "cursor no longer valid; restart the query".to_string(),
            ));
        }

        return Ok(QueryPlan {
            db_id: state.db_id.clone(),
            sql: state.sql.clone(),
            params: state.params.clone(),
            max_rows: state.max_rows,
            max_bytes: state.max_bytes,
            existing_cursor: Some(state),
            existing_cursor_id: Some(cursor),
        });
    }

    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let sql = request.sql.ok_or_else(|| {
        AppError::InvalidInput("sql is required when cursor is omitted".to_string())
    })?;
    let max_rows = request.max_rows.unwrap_or(policy.max_rows);
    let max_bytes = request.max_bytes.unwrap_or(policy.max_bytes);
    if max_rows == 0 {
        return Err(AppError::InvalidInput(
            "max_rows must be greater than zero".to_string(),
        ));
    }
    if max_bytes == 0 {
        return Err(AppError::InvalidInput(
            "max_bytes must be greater than zero".to_string(),
        ));
    }

    Ok(QueryPlan {
        db_id,
        sql,
        params: request.params,
        max_rows,
        max_bytes,
        existing_cursor: None,
        existing_cursor_id: None,
    })
}

struct QueryPlan {
    db_id: String,
    sql: String,
    params: Option<SqlParams>,
    max_rows: usize,
    max_bytes: usize,
    existing_cursor: Option<CursorState>,
    existing_cursor_id: Option<String>,
}

fn row_to_json_map(row: &rusqlite::Row<'_>) -> AppResult<Map<String, Value>> {
    let mut mapped = Map::new();
    for index in 0..row.as_ref().column_count() {
        let column_name = row
            .as_ref()
            .column_name(index)
            .map(|name| name.to_string())
            .unwrap_or_else(|_| format!("column_{index}"));
        let value = row.get_ref(index)?;
        mapped.insert(column_name, sql_value_ref_to_json(value));
    }
    Ok(mapped)
}

fn sql_value_ref_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::Number(number.into()),
        ValueRef::Real(number) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(bytes) => Value::String(String::from_utf8_lossy(bytes).to_string()),
        ValueRef::Blob(bytes) => Value::String(format!("0x{}", hex_encode(bytes))),
    }
}

fn bind_params(statement: &mut Statement<'_>, params: Option<&SqlParams>) -> AppResult<()> {
    let Some(params) = params else {
        return Ok(());
    };

    match params {
        SqlParams::Positional(values) => {
            let expected = statement.parameter_count();
            if values.len() != expected {
                return Err(AppError::InvalidInput(format!(
                    "expected {expected} positional parameters, got {}",
                    values.len()
                )));
            }

            for (index, value) in values.iter().enumerate() {
                statement.raw_bind_parameter(index + 1, json_to_sql_value(value)?)?;
            }
        }
        SqlParams::Named(values) => {
            for (name, value) in values {
                let normalized =
                    if name.starts_with(':') || name.starts_with('@') || name.starts_with('$') {
                        name.clone()
                    } else {
                        format!(":{name}")
                    };
                let Some(index) = statement.parameter_index(&normalized)? else {
                    return Err(AppError::InvalidInput(format!(
                        "unknown named parameter: {name}"
                    )));
                };
                statement.raw_bind_parameter(index, json_to_sql_value(value)?)?;
            }
        }
    }

    Ok(())
}

fn json_to_sql_value(value: &Value) -> AppResult<SqlValue> {
    let mapped = match value {
        Value::Null => SqlValue::Null,
        Value::Bool(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Number(number) => {
            if let Some(as_i64) = number.as_i64() {
                SqlValue::Integer(as_i64)
            } else if let Some(as_f64) = number.as_f64() {
                SqlValue::Real(as_f64)
            } else {
                return Err(AppError::InvalidInput(
                    "numeric parameter is out of range".to_string(),
                ));
            }
        }
        Value::String(text) => SqlValue::Text(text.clone()),
        Value::Array(_) | Value::Object(_) => SqlValue::Text(value.to_string()),
    };
    Ok(mapped)
}

fn fingerprint_query(
    db_id: &str,
    sql: &str,
    params: Option<&SqlParams>,
    max_rows: usize,
    max_bytes: usize,
) -> AppResult<String> {
    let payload = json!({
        "db_id": db_id,
        "sql": sql,
        "params": params,
        "max_rows": max_rows,
        "max_bytes": max_bytes,
    });
    let serialized = serde_json::to_string(&payload).map_err(|error| {
        AppError::Dependency(format!("failed to hash query fingerprint payload: {error}"))
    })?;

    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
