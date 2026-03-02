use std::time::Instant;

use csv::StringRecord;
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use crate::DEFAULT_DB_ID;
use crate::contracts::common::ToolEnvelope;
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

    Ok(finalize_tool(
        "Import completed.",
        DbImportData {
            table: request.table,
            columns,
            rows_inserted: inserted,
            rows_skipped: skipped,
        },
        started,
        Vec::new(),
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
        json_rows[0].keys().cloned().collect::<Vec<_>>()
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

    format!(
        "{conflict} {quoted_table} ({column_list}) VALUES ({placeholders})",
    )
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
