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

    if request.reset {
        cursor_store.invalidate_db(&db_id);
    }

    let result = registry.open_db(
        db_id,
        request.mode,
        request.path,
        request.reset,
        persist_root,
        max_db_bytes,
    )?;
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
