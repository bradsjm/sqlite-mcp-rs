use std::time::Instant;

use crate::DEFAULT_DB_ID;
use crate::contracts::common::ToolEnvelope;
use crate::contracts::db::{
    DbCloseData, DbCloseRequest, DbListData, DbListRequest, DbOpenData, DbOpenRequest,
};
use crate::db::registry::DbRegistry;
use crate::errors::AppResult;
use crate::pagination::cursor_store::CursorStore;
use crate::server::finalize::finalize_tool;

pub fn db_open(
    registry: &mut DbRegistry,
    cursor_store: &mut CursorStore,
    request: DbOpenRequest,
    persist_root: Option<&std::path::Path>,
    max_db_bytes: u64,
) -> AppResult<ToolEnvelope<DbOpenData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let reset = request.reset;
    let result = registry.open_db(
        db_id.clone(),
        request.mode,
        request.path,
        reset,
        persist_root,
        max_db_bytes,
    )?;
    if reset {
        cursor_store.invalidate_db(&db_id);
    }
    Ok(finalize_tool(
        "Database opened.",
        result,
        started,
        Vec::new(),
        None,
        None,
    ))
}

pub fn db_close(
    registry: &mut DbRegistry,
    cursor_store: &mut CursorStore,
    request: DbCloseRequest,
) -> AppResult<ToolEnvelope<DbCloseData>> {
    let started = Instant::now();
    let closed = registry.close_db(request.db_id.as_deref())?;
    cursor_store.invalidate_db(&closed.db_id);
    Ok(finalize_tool(
        "Database closed.",
        closed,
        started,
        Vec::new(),
        None,
        None,
    ))
}

pub fn db_list(
    registry: &DbRegistry,
    _request: DbListRequest,
    persist_root: Option<&std::path::Path>,
    persisted_limit: usize,
) -> AppResult<ToolEnvelope<DbListData>> {
    let started = Instant::now();
    let listed = registry.list(persist_root, persisted_limit)?;
    Ok(finalize_tool(
        "Listed database handles.",
        listed,
        started,
        Vec::new(),
        None,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::contracts::db::{DbMode, DbOpenRequest};
    use crate::pagination::cursor_store::CursorState;

    use super::{CursorStore, DbRegistry, db_open};

    fn cursor_state() -> CursorState {
        CursorState {
            db_id: "default".to_string(),
            fingerprint: "select-1".to_string(),
            offset: 0,
            sql: "SELECT 1".to_string(),
            params: None,
            max_rows: 100,
            max_bytes: 65_536,
        }
    }

    #[test]
    fn failed_reset_preserves_cursor_and_successful_reset_invalidates_it() {
        let mut registry = DbRegistry::default();
        let mut cursor_store = CursorStore::new(Duration::from_secs(60), 10);
        db_open(
            &mut registry,
            &mut cursor_store,
            DbOpenRequest {
                db_id: None,
                mode: DbMode::Memory,
                path: None,
                reset: false,
            },
            None,
            100_000_000,
        )
        .expect("initial open should succeed");
        let cursor = cursor_store
            .create(cursor_state())
            .expect("cursor storage should be enabled");

        assert!(
            db_open(
                &mut registry,
                &mut cursor_store,
                DbOpenRequest {
                    db_id: None,
                    mode: DbMode::Persist,
                    path: Some("replacement.sqlite".to_string()),
                    reset: true,
                },
                None,
                100_000_000,
            )
            .is_err()
        );
        assert!(
            cursor_store.get(&cursor).is_some(),
            "failed reset must not invalidate existing cursors"
        );

        db_open(
            &mut registry,
            &mut cursor_store,
            DbOpenRequest {
                db_id: None,
                mode: DbMode::Memory,
                path: None,
                reset: true,
            },
            None,
            100_000_000,
        )
        .expect("successful reset should succeed");
        assert!(
            cursor_store.get(&cursor).is_none(),
            "successful reset must invalidate existing cursors"
        );
    }
}
