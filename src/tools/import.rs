use std::collections::BTreeSet;
use std::time::Instant;

use csv::StringRecord;
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use crate::DEFAULT_DB_ID;
use crate::contracts::common::{ToolEnvelope, ToolHint};
use crate::contracts::import::{
    DbImportData, DbImportRequest, ImportConflictMode, ImportFormat, ImportPayload,
};
use crate::db::persistence::enforce_db_size_limit;
use crate::db::registry::DbRegistry;
use crate::errors::{AppError, AppResult};
use crate::policy::{SqlPolicy, is_valid_identifier};
use crate::server::finalize::finalize_tool;

pub fn db_import(
    registry: &DbRegistry,
    policy: &SqlPolicy,
    request: DbImportRequest,
) -> AppResult<ToolEnvelope<DbImportData>> {
    let started = Instant::now();
    let db_id = request
        .db_id
        .clone()
        .unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let connection = registry.get_connection(Some(&db_id))?;
    let persisted_path = registry.persisted_path(Some(&db_id))?;

    if !is_valid_identifier(&request.table) {
        return Err(AppError::InvalidInput(
            "table must match ^[A-Za-z_][A-Za-z0-9_]*$".to_string(),
        ));
    }

    if request.batch_size == Some(0) {
        return Err(AppError::InvalidInput(
            "batch_size must be greater than zero".to_string(),
        ));
    }
    let effective_batch_size = request
        .batch_size
        .unwrap_or(rows_batch_default(policy.max_rows));

    let payload_bytes = estimate_payload_bytes(&request.data)?;
    if payload_bytes > policy.max_bytes {
        return Err(AppError::LimitExceeded(format!(
            "import payload exceeds max_bytes ({})",
            policy.max_bytes
        )));
    }

    let ParsedImport { columns, rows } = parse_import_rows(request.format, &request)?;
    if rows.is_empty() {
        return Err(AppError::InvalidInput(
            "import payload does not contain rows".to_string(),
        ));
    }
    if columns.is_empty() {
        return Err(AppError::InvalidInput(
            "import requires at least one column".to_string(),
        ));
    }
    if rows.len() > policy.max_rows {
        return Err(AppError::LimitExceeded(format!(
            "import row count exceeds max_rows ({})",
            policy.max_rows
        )));
    }

    for column in &columns {
        if !is_valid_identifier(column) {
            return Err(AppError::InvalidInput(
                "column names must match ^[A-Za-z_][A-Za-z0-9_]*$".to_string(),
            ));
        }
    }

    let table_exists = import_table_exists(connection, &request.table)?;
    if !table_exists {
        if request.create_table_if_missing {
            create_import_table(
                connection,
                &request.table,
                &columns,
                &rows,
                request.infer_column_types,
            )?;
        } else {
            return Err(AppError::NotFound(format!(
                "table {} does not exist; set create_table_if_missing=true to create it",
                request.table
            )));
        }
    }

    connection.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
    if request.truncate_first {
        let truncate_sql = format!("DELETE FROM {}", quote_identifier(&request.table));
        if let Err(error) = connection.execute(&truncate_sql, []) {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error.into());
        }
    }

    let mut inserted = 0usize;
    let mut skipped = 0usize;
    let sql = build_insert_sql(&request.table, &columns, request.on_conflict);
    for (index, row) in rows.iter().enumerate() {
        let values = row
            .iter()
            .map(json_to_sql_value)
            .collect::<AppResult<Vec<_>>>()?;

        match connection.execute(&sql, rusqlite::params_from_iter(values)) {
            Ok(affected) => {
                if affected == 0 {
                    skipped += 1;
                } else {
                    inserted += affected;
                }
            }
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK");
                return Err(error.into());
            }
        }

        if (index + 1) % effective_batch_size == 0
            && let Err(error) =
                enforce_db_size_limit(persisted_path.as_deref(), policy.max_db_bytes)
        {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error);
        }
    }

    if let Err(error) = enforce_db_size_limit(persisted_path.as_deref(), policy.max_db_bytes) {
        let _ = connection.execute_batch("ROLLBACK");
        return Err(error);
    }

    connection.execute_batch("COMMIT")?;

    let mut hints = Vec::new();
    hints.push(ToolHint {
        tool: "sql_query".to_string(),
        arguments: serde_json::json!({
            "db_id": db_id,
            "sql": format!("SELECT * FROM {} LIMIT 50", quote_identifier(&request.table)),
        }),
        reason: "Preview imported rows from the destination table.".to_string(),
    });

    Ok(finalize_tool(
        "Import completed.",
        DbImportData {
            table: request.table,
            columns,
            rows_inserted: inserted,
            rows_skipped: skipped,
        },
        started,
        hints,
        None,
        None,
    ))
}

struct ParsedImport {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

fn parse_import_rows(format: ImportFormat, request: &DbImportRequest) -> AppResult<ParsedImport> {
    match format {
        ImportFormat::Csv => parse_csv_rows(request),
        ImportFormat::Json => parse_json_rows(request),
    }
}

fn parse_csv_rows(request: &DbImportRequest) -> AppResult<ParsedImport> {
    let csv_text = match &request.data {
        ImportPayload::Text(value) => value.as_str(),
        ImportPayload::JsonRows(_) => {
            return Err(AppError::InvalidInput(
                "csv imports require data to be a string".to_string(),
            ));
        }
    };

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(csv_text.as_bytes());

    let headers = reader
        .headers()
        .map_err(|error| AppError::InvalidInput(format!("invalid csv headers: {error}")))?
        .clone();

    let columns = if request.columns.is_empty() {
        headers
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
    } else {
        request.columns.clone()
    };

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record
            .map_err(|error| AppError::InvalidInput(format!("invalid csv record: {error}")))?;
        rows.push(record_to_row(&columns, &headers, &record));
    }

    Ok(ParsedImport { columns, rows })
}

fn parse_json_rows(request: &DbImportRequest) -> AppResult<ParsedImport> {
    let json_rows = match &request.data {
        ImportPayload::Text(value) => {
            serde_json::from_str::<Vec<serde_json::Map<String, Value>>>(value).map_err(|error| {
                AppError::InvalidInput(format!(
                    "json imports require valid JSON row array: {error}"
                ))
            })?
        }
        ImportPayload::JsonRows(rows) => rows.clone(),
    };

    if json_rows.is_empty() {
        return Ok(ParsedImport {
            columns: request.columns.clone(),
            rows: Vec::new(),
        });
    }

    let columns = if request.columns.is_empty() {
        let mut discovered = BTreeSet::new();
        for row in &json_rows {
            discovered.extend(row.keys().cloned());
        }
        discovered.into_iter().collect::<Vec<_>>()
    } else {
        request.columns.clone()
    };

    let mut parsed_rows = Vec::with_capacity(json_rows.len());
    for object in json_rows {
        let parsed = columns
            .iter()
            .map(|column| object.get(column).cloned().unwrap_or(Value::Null))
            .collect::<Vec<_>>();
        parsed_rows.push(parsed);
    }

    Ok(ParsedImport {
        columns,
        rows: parsed_rows,
    })
}

fn record_to_row(columns: &[String], headers: &StringRecord, record: &StringRecord) -> Vec<Value> {
    columns
        .iter()
        .map(|column| {
            if let Some(index) = headers.iter().position(|header| header == column) {
                Value::String(record.get(index).unwrap_or_default().to_string())
            } else {
                Value::Null
            }
        })
        .collect()
}

fn build_insert_sql(
    table: &str,
    columns: &[String],
    on_conflict: Option<ImportConflictMode>,
) -> String {
    let conflict = match on_conflict.unwrap_or(ImportConflictMode::None) {
        ImportConflictMode::None => "INSERT INTO",
        ImportConflictMode::Ignore => "INSERT OR IGNORE INTO",
        ImportConflictMode::Replace => "INSERT OR REPLACE INTO",
    };

    let quoted_table = quote_identifier(table);
    let column_list = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = std::iter::repeat_n("?", columns.len())
        .collect::<Vec<_>>()
        .join(", ");

    format!("{conflict} {quoted_table} ({column_list}) VALUES ({placeholders})",)
}

fn quote_identifier(identifier: &str) -> String {
    let escaped = identifier.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn estimate_payload_bytes(value: &ImportPayload) -> AppResult<usize> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| AppError::Dependency(format!("failed to encode import payload: {error}")))
}

fn json_to_sql_value(value: &Value) -> AppResult<SqlValue> {
    let converted = match value {
        Value::Null => SqlValue::Null,
        Value::Bool(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Number(number) => {
            if let Some(as_i64) = number.as_i64() {
                SqlValue::Integer(as_i64)
            } else if let Some(as_f64) = number.as_f64() {
                SqlValue::Real(as_f64)
            } else {
                return Err(AppError::InvalidInput(
                    "numeric import value is out of range".to_string(),
                ));
            }
        }
        Value::String(text) => SqlValue::Text(text.clone()),
        Value::Array(_) | Value::Object(_) => SqlValue::Text(value.to_string()),
    };
    Ok(converted)
}

fn rows_batch_default(max_rows: usize) -> usize {
    max_rows.clamp(1, 1000)
}

fn import_table_exists(connection: &rusqlite::Connection, table: &str) -> AppResult<bool> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1 LIMIT 1",
            rusqlite::params![table],
            |row| row.get::<_, i64>(0),
        )
        .map(|_| true)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(other.into()),
        })
}

fn create_import_table(
    connection: &rusqlite::Connection,
    table: &str,
    columns: &[String],
    rows: &[Vec<Value>],
    infer_column_types: bool,
) -> AppResult<()> {
    let mut column_defs = Vec::with_capacity(columns.len());
    for (index, column) in columns.iter().enumerate() {
        let ty = if infer_column_types {
            infer_column_type(rows, index)
        } else {
            "TEXT"
        };
        column_defs.push(format!("{} {}", quote_identifier(column), ty));
    }
    let create_sql = format!(
        "CREATE TABLE {} ({})",
        quote_identifier(table),
        column_defs.join(", ")
    );
    connection.execute(&create_sql, [])?;
    Ok(())
}

fn infer_column_type(rows: &[Vec<Value>], index: usize) -> &'static str {
    let mut saw_real = false;
    let mut saw_integer = false;
    let mut saw_bool = false;
    let mut saw_textual = false;

    for row in rows {
        let Some(value) = row.get(index) else {
            continue;
        };
        match value {
            Value::Null => {}
            Value::Bool(_) => saw_bool = true,
            Value::Number(number) => {
                if number.as_i64().is_some() {
                    saw_integer = true;
                } else {
                    saw_real = true;
                }
            }
            Value::String(_) | Value::Array(_) | Value::Object(_) => saw_textual = true,
        }
    }

    if saw_textual {
        "TEXT"
    } else if saw_real {
        "REAL"
    } else if saw_integer || saw_bool {
        "INTEGER"
    } else {
        "TEXT"
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value, json};

    use crate::contracts::db::DbMode;
    use crate::contracts::import::{DbImportRequest, ImportFormat, ImportPayload};
    use crate::db::registry::DbRegistry;
    use crate::errors::AppError;
    use crate::policy::SqlPolicy;

    use super::db_import;

    fn test_policy() -> SqlPolicy {
        SqlPolicy {
            max_sql_length: 20_000,
            max_statements: 50,
            max_rows: 500,
            max_bytes: 1_048_576,
            max_db_bytes: u64::MAX,
        }
    }

    fn setup_registry() -> DbRegistry {
        let mut registry = DbRegistry::default();
        registry
            .open_db(
                "default".to_string(),
                DbMode::Memory,
                None,
                false,
                None,
                u64::MAX,
            )
            .expect("memory db should open");
        registry
    }

    fn json_rows(rows: &[Value]) -> ImportPayload {
        let mut mapped = Vec::new();
        for row in rows {
            let object = row
                .as_object()
                .cloned()
                .expect("test rows must be JSON objects");
            let map: Map<String, Value> = object.into_iter().collect();
            mapped.push(map);
        }
        ImportPayload::JsonRows(mapped)
    }

    #[test]
    fn import_auto_creates_table_with_union_of_json_keys() {
        let registry = setup_registry();
        let response = db_import(
            &registry,
            &test_policy(),
            DbImportRequest {
                db_id: None,
                format: ImportFormat::Json,
                table: "imported_items".to_string(),
                columns: Vec::new(),
                data: json_rows(&[json!({"id": 1}), json!({"name": "alpha"})]),
                batch_size: None,
                on_conflict: None,
                truncate_first: false,
                create_table_if_missing: true,
                infer_column_types: true,
            },
        )
        .expect("import should succeed");

        assert_eq!(
            response.data.columns,
            vec!["id".to_string(), "name".to_string()]
        );
        assert_eq!(response.data.rows_inserted, 2);

        let connection = registry
            .get_connection(Some("default"))
            .expect("default db should exist");
        let mut stmt = connection
            .prepare("SELECT id, name FROM imported_items ORDER BY rowid ASC")
            .expect("query should prepare");
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .expect("query should execute")
            .collect::<Result<Vec<_>, _>>()
            .expect("rows should decode");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (Some(1), None));
        assert_eq!(rows[1], (None, Some("alpha".to_string())));
    }

    #[test]
    fn import_missing_table_requires_opt_in_when_auto_create_disabled() {
        let registry = setup_registry();
        let error = db_import(
            &registry,
            &test_policy(),
            DbImportRequest {
                db_id: None,
                format: ImportFormat::Json,
                table: "missing_table".to_string(),
                columns: Vec::new(),
                data: json_rows(&[json!({"value": 1})]),
                batch_size: None,
                on_conflict: None,
                truncate_first: false,
                create_table_if_missing: false,
                infer_column_types: true,
            },
        )
        .expect_err("missing table must fail when auto create is disabled");

        match error {
            AppError::NotFound(message) => {
                assert!(message.contains("create_table_if_missing=true"));
            }
            other => panic!("expected not found error, got: {other}"),
        }
    }
}
