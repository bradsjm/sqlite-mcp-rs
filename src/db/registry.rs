//! Database registry for managing SQLite connections.
//!
//! The registry maintains a collection of open database handles and provides
//! operations for opening, closing, and listing databases.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "vector")]
use std::sync::OnceLock;

use rusqlite::Connection;
#[cfg(feature = "vector")]
use sqlite_vec::sqlite3_vec_init;

use crate::DEFAULT_DB_ID;
use crate::contracts::db::{
    DbCloseData, DbListData, DbMode, DbOpenData, DbSummary, ExtensionsLoaded,
};
use crate::errors::{AppError, AppResult};

use super::persistence::{enforce_db_size_limit, list_persisted_entries, resolve_persist_path};

/// Internal handle for an open database connection.
#[derive(Debug)]
struct DbHandle {
    /// SQLite connection.
    connection: Connection,
    /// Storage mode (memory or persisted).
    mode: DbMode,
    /// File path for persisted databases.
    path: Option<PathBuf>,
}

/// Registry for managing open database connections.
///
/// Maintains a map of database identifiers to their connections,
/// with one database designated as "active" for operations that
/// don't specify a database explicitly.
#[derive(Debug)]
pub struct DbRegistry {
    /// Currently active database identifier.
    active_db_id: String,
    /// Map of database identifiers to their handles.
    handles: HashMap<String, DbHandle>,
}

impl Default for DbRegistry {
    fn default() -> Self {
        Self {
            active_db_id: DEFAULT_DB_ID.to_string(),
            handles: HashMap::new(),
        }
    }
}

impl DbRegistry {
    /// Opens a database connection with the specified parameters.
    ///
    /// If the database is already open with different parameters and `reset` is false,
    /// returns a conflict. When `reset` is true, replaces the existing connection after the
    /// replacement has opened and passed its limits.
    ///
    /// # Arguments
    ///
    /// * `db_id` - Unique identifier for the database
    /// * `mode` - Storage mode (memory or persisted)
    /// * `path` - File path for persisted databases (relative to persist_root)
    /// * `reset` - Whether to replace an existing open database
    /// * `persist_root` - Root directory for persisted databases
    /// * `max_db_bytes` - Maximum allowed database file size
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Conflict`] if database is open with different parameters
    /// and reset is false. Returns various errors for filesystem or SQLite issues.
    pub fn open_db(
        &mut self,
        db_id: String,
        mode: DbMode,
        path: Option<String>,
        reset: bool,
        persist_root: Option<&Path>,
        max_db_bytes: u64,
    ) -> AppResult<DbOpenData> {
        #[cfg(feature = "vector")]
        ensure_sqlite_vec_registered()?;

        if let Some(existing) = self.handles.get(&db_id) {
            let requested_path = path.clone().map(PathBuf::from);
            if !reset && (existing.mode != mode || existing.path != requested_path) {
                return Err(AppError::Conflict(
                    "db_id already open with different mode or path; set reset=true to replace"
                        .to_string(),
                ));
            }
            if !reset {
                self.active_db_id = db_id.clone();
                return Ok(DbOpenData {
                    db_id,
                    mode,
                    path,
                    active: true,
                    extensions_loaded: extension_flags(&existing.connection),
                });
            }
        }

        let (connection, resolved_path) = match mode {
            DbMode::Memory => (Connection::open_in_memory()?, None),
            DbMode::Persist => {
                let root = persist_root.ok_or_else(|| {
                    AppError::ConfigMissing(
                        "SQLITE_PERSIST_ROOT is required for persist mode".into(),
                    )
                })?;
                let requested_path = path.clone().ok_or_else(|| {
                    AppError::InvalidInput("path is required when mode=persist".to_string())
                })?;
                let resolved = resolve_persist_path(root, &requested_path)?;
                if let Some(parent) = resolved.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        AppError::Dependency(format!(
                            "failed creating persistence directory: {error}"
                        ))
                    })?;
                }
                (Connection::open(&resolved)?, Some(resolved))
            }
        };

        apply_db_size_limit(&connection, max_db_bytes)?;

        let extensions_loaded = extension_flags(&connection);

        enforce_db_size_limit(resolved_path.as_deref(), max_db_bytes)?;

        self.handles.insert(
            db_id.clone(),
            DbHandle {
                connection,
                mode,
                path: resolved_path.clone(),
            },
        );
        self.active_db_id = db_id.clone();

        Ok(DbOpenData {
            db_id,
            mode,
            path: resolved_path.map(|path| path.to_string_lossy().to_string()),
            active: true,
            extensions_loaded,
        })
    }

    /// Closes a database connection.
    ///
    /// If no database ID is specified, closes the active database.
    /// Updates the active database to another open database if available.
    ///
    /// # Arguments
    ///
    /// * `db_id` - Optional database identifier (defaults to active database)
    ///
    /// # Errors
    ///
    /// Returns [`AppError::NotFound`] if the database is not open.
    pub fn close_db(&mut self, db_id: Option<&str>) -> AppResult<DbCloseData> {
        let resolved_db_id = db_id.unwrap_or(&self.active_db_id).to_string();
        if self.handles.remove(&resolved_db_id).is_none() {
            return Err(AppError::NotFound(format!(
                "unknown db_id: {resolved_db_id}"
            )));
        }

        if self.handles.is_empty() {
            self.active_db_id = DEFAULT_DB_ID.to_string();
        } else if self.active_db_id == resolved_db_id {
            if self.handles.contains_key(DEFAULT_DB_ID) {
                self.active_db_id = DEFAULT_DB_ID.to_string();
            } else if let Some(next_db_id) = self.handles.keys().min().cloned() {
                self.active_db_id = next_db_id;
            }
        }

        Ok(DbCloseData {
            db_id: resolved_db_id,
            closed: true,
            active_db_id: self.active_db_id.clone(),
        })
    }

    /// Lists open databases and persisted database files.
    ///
    /// Returns information about all open database handles and scans
    /// the persistence root for persisted database files.
    ///
    /// # Arguments
    ///
    /// * `persist_root` - Root directory for persisted databases
    /// * `persisted_limit` - Maximum number of persisted files to list
    pub fn list(
        &self,
        persist_root: Option<&Path>,
        persisted_limit: usize,
    ) -> AppResult<DbListData> {
        let open = self
            .handles
            .iter()
            .map(|(db_id, handle)| DbSummary {
                db_id: db_id.clone(),
                mode: handle.mode,
                path: handle
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string()),
            })
            .collect::<Vec<_>>();

        let (persisted, persisted_truncated) = match persist_root {
            Some(root) => list_persisted_entries(root, persisted_limit)?,
            None => (Vec::new(), false),
        };

        Ok(DbListData {
            active_db_id: self.active_db_id.clone(),
            open,
            persisted,
            persisted_truncated,
        })
    }

    /// Returns a reference to the SQLite connection for the specified database.
    ///
    /// # Arguments
    ///
    /// * `db_id` - Optional database identifier (defaults to active database)
    ///
    /// # Errors
    ///
    /// Returns [`AppError::NotFound`] if the database is not open.
    pub fn get_connection(&self, db_id: Option<&str>) -> AppResult<&Connection> {
        let resolved_db_id = db_id.unwrap_or(&self.active_db_id);
        self.handles
            .get(resolved_db_id)
            .map(|handle| &handle.connection)
            .ok_or_else(|| AppError::NotFound(format!("unknown db_id: {resolved_db_id}")))
    }

    /// Returns the file path for a persisted database.
    ///
    /// # Arguments
    ///
    /// * `db_id` - Optional database identifier (defaults to active database)
    ///
    /// # Returns
    ///
    /// `Some(PathBuf)` for persisted databases, `None` for in-memory databases.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::NotFound`] if the database is not open.
    pub fn persisted_path(&self, db_id: Option<&str>) -> AppResult<Option<PathBuf>> {
        let resolved_db_id = db_id.unwrap_or(&self.active_db_id);
        let handle = self
            .handles
            .get(resolved_db_id)
            .ok_or_else(|| AppError::NotFound(format!("unknown db_id: {resolved_db_id}")))?;
        Ok(handle.path.clone())
    }
}

/// Caps SQLite's page allocation so database growth cannot exceed the configured byte limit.
fn apply_db_size_limit(connection: &Connection, max_db_bytes: u64) -> AppResult<()> {
    let page_size = connection.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))?;
    let page_size = u64::try_from(page_size).map_err(|_| {
        AppError::LimitExceeded("SQLite returned a non-positive page_size".to_string())
    })?;
    if page_size == 0 {
        return Err(AppError::LimitExceeded(
            "SQLite returned a zero page_size".to_string(),
        ));
    }
    if max_db_bytes < page_size {
        return Err(AppError::LimitExceeded(format!(
            "max_db_bytes ({max_db_bytes}) is smaller than SQLite page_size ({page_size}); increase max_db_bytes to at least {page_size}"
        )));
    }

    let max_pages = max_db_bytes / page_size;
    let effective_pages =
        connection.query_row(&format!("PRAGMA max_page_count = {max_pages}"), [], |row| {
            row.get::<_, i64>(0)
        })?;
    let effective_pages = u64::try_from(effective_pages).map_err(|_| {
        AppError::LimitExceeded("SQLite returned a negative max_page_count".to_string())
    })?;
    if effective_pages > max_pages {
        return Err(AppError::LimitExceeded(format!(
            "SQLite requires {effective_pages} pages, exceeding the configured ceiling of {max_pages} pages ({max_db_bytes} bytes)"
        )));
    }

    Ok(())
}

#[cfg(feature = "vector")]
fn extension_flags(connection: &Connection) -> ExtensionsLoaded {
    let vec_loaded = {
        connection
            .query_row("select vec_version()", [], |row| row.get::<_, String>(0))
            .is_ok()
    };

    ExtensionsLoaded {
        vec: vec_loaded,
        rembed: false,
        regex: false,
    }
}

#[cfg(not(feature = "vector"))]
fn extension_flags(_connection: &Connection) -> ExtensionsLoaded {
    ExtensionsLoaded {
        vec: false,
        rembed: false,
        regex: false,
    }
}

#[cfg(feature = "vector")]
fn ensure_sqlite_vec_registered() -> AppResult<()> {
    static REGISTER_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

    let result = REGISTER_RESULT.get_or_init(|| {
        type SqliteExtensionEntry = unsafe extern "C" fn(
            db: *mut rusqlite::ffi::sqlite3,
            pz_err_msg: *mut *mut std::os::raw::c_char,
            api: *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        let rc = unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                SqliteExtensionEntry,
            >(sqlite3_vec_init as *const ())))
        };
        if rc == rusqlite::ffi::SQLITE_OK {
            Ok(())
        } else {
            Err(format!(
                "failed to register sqlite-vec auto extension (sqlite rc={rc})"
            ))
        }
    });

    result
        .clone()
        .map_err(|message| AppError::Dependency(message.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::contracts::db::DbMode;
    use crate::errors::AppError;
    use rusqlite::Connection;

    use super::{DbRegistry, apply_db_size_limit};

    #[test]
    fn opens_memory_db() {
        let mut registry = DbRegistry::default();
        let opened = registry
            .open_db(
                "default".to_string(),
                DbMode::Memory,
                None,
                false,
                None,
                100_000_000,
            )
            .expect("db should open");
        assert_eq!(opened.db_id, "default");
        let listed = registry.list(None, 100).expect("list should succeed");
        assert_eq!(listed.open.len(), 1);
    }

    #[test]
    fn memory_db_page_cap_rejects_oversized_write() {
        let mut registry = DbRegistry::default();
        registry
            .open_db(
                "bounded".to_string(),
                DbMode::Memory,
                None,
                false,
                None,
                8_192,
            )
            .expect("db should open");

        let connection = registry
            .get_connection(Some("bounded"))
            .expect("bounded connection should exist");
        let max_page_count: i64 = connection
            .query_row("PRAGMA max_page_count", [], |row| row.get(0))
            .expect("max page count should be readable");
        assert!(max_page_count <= 2);
        connection
            .execute_batch("CREATE TABLE data (value BLOB)")
            .expect("schema should fit within the page cap");
        assert!(
            connection
                .execute("INSERT INTO data VALUES (zeroblob(1048576))", [])
                .is_err(),
            "oversized write should be rejected by SQLite"
        );
    }

    #[test]
    fn rejects_sub_page_database_limit() {
        let connection = Connection::open_in_memory().expect("connection should open");
        let page_size: i64 = connection
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .expect("page size should be readable");
        let page_size = u64::try_from(page_size).expect("page size should be positive");

        let error = apply_db_size_limit(&connection, page_size - 1)
            .expect_err("a limit smaller than one page must be rejected");
        assert!(matches!(error, AppError::LimitExceeded(_)));
        assert!(error.to_string().contains("page_size"));
    }

    #[test]
    fn rejects_effective_page_count_above_configured_ceiling() {
        let connection = Connection::open_in_memory().expect("connection should open");
        connection
            .execute_batch(
                "CREATE TABLE data (value BLOB);
                 INSERT INTO data VALUES (zeroblob(1048576));",
            )
            .expect("database should grow beyond one page");
        let page_size: i64 = connection
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .expect("page size should be readable");
        let page_size = u64::try_from(page_size).expect("page size should be positive");
        let page_count: i64 = connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .expect("page count should be readable");
        assert!(page_count > 1, "fixture must exceed one page");

        let error = apply_db_size_limit(&connection, page_size)
            .expect_err("SQLite must not silently retain a larger page allowance");
        assert!(matches!(error, AppError::LimitExceeded(_)));
        let effective_max: i64 = connection
            .query_row("PRAGMA max_page_count", [], |row| row.get(0))
            .expect("effective page ceiling should be readable");
        assert!(effective_max > 1);
    }

    #[test]
    fn failed_reset_with_sub_page_limit_preserves_existing_connection() {
        let mut registry = DbRegistry::default();
        registry
            .open_db(
                "default".to_string(),
                DbMode::Memory,
                None,
                false,
                None,
                100_000_000,
            )
            .expect("db should open");
        registry
            .get_connection(Some("default"))
            .expect("connection should exist")
            .execute_batch("CREATE TABLE data (value INTEGER); INSERT INTO data VALUES (7)")
            .expect("seed data should be stored");

        let error = registry
            .open_db("default".to_string(), DbMode::Memory, None, true, None, 1)
            .expect_err("sub-page reset must fail before replacing the existing handle");
        assert!(matches!(error, AppError::LimitExceeded(_)));

        let preserved = registry
            .get_connection(Some("default"))
            .expect("failed reset must preserve the existing handle");
        let value: i64 = preserved
            .query_row("SELECT value FROM data", [], |row| row.get(0))
            .expect("seed data should remain queryable");
        assert_eq!(value, 7);
    }

    #[test]
    fn failed_reset_preserves_existing_connection() {
        let mut registry = DbRegistry::default();
        registry
            .open_db(
                "default".to_string(),
                DbMode::Memory,
                None,
                false,
                None,
                100_000_000,
            )
            .expect("db should open");
        let connection = registry
            .get_connection(Some("default"))
            .expect("connection should exist");
        connection
            .execute_batch("CREATE TABLE data (value INTEGER); INSERT INTO data VALUES (7)")
            .expect("seed data should be stored");

        assert!(
            registry
                .open_db(
                    "default".to_string(),
                    DbMode::Persist,
                    Some("replacement.sqlite".to_string()),
                    true,
                    None,
                    100_000_000,
                )
                .is_err()
        );

        let preserved = registry
            .get_connection(Some("default"))
            .expect("failed reset must preserve the existing handle");
        let value: i64 = preserved
            .query_row("SELECT value FROM data", [], |row| row.get(0))
            .expect("seed data should remain queryable");
        assert_eq!(value, 7);
    }

    #[test]
    fn rejects_persist_without_root() {
        let mut registry = DbRegistry::default();
        let result = registry.open_db(
            "persist".to_string(),
            DbMode::Persist,
            Some("db.sqlite".to_string()),
            false,
            None,
            100_000_000,
        );
        assert!(result.is_err());
    }

    #[test]
    fn opens_persist_with_root() {
        let mut registry = DbRegistry::default();
        let root = Path::new("/tmp/sqlite-mcp-rs-tests");
        let result = registry.open_db(
            "persist".to_string(),
            DbMode::Persist,
            Some("main.db".to_string()),
            true,
            Some(root),
            100_000_000,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn allows_opening_same_persist_path_with_multiple_db_ids() {
        let mut registry = DbRegistry::default();
        let root = Path::new("/tmp/sqlite-mcp-rs-tests");

        let first = registry.open_db(
            "a".to_string(),
            DbMode::Persist,
            Some("shared.db".to_string()),
            true,
            Some(root),
            100_000_000,
        );
        assert!(first.is_ok());

        let second = registry.open_db(
            "b".to_string(),
            DbMode::Persist,
            Some("shared.db".to_string()),
            false,
            Some(root),
            100_000_000,
        );
        assert!(second.is_ok());

        let listed = registry
            .list(Some(root), 100)
            .expect("list should include both db handles");
        assert_eq!(listed.open.len(), 2);
    }

    #[test]
    fn closes_open_db() {
        let mut registry = DbRegistry::default();
        registry
            .open_db(
                "default".to_string(),
                DbMode::Memory,
                None,
                false,
                None,
                100_000_000,
            )
            .expect("db should open");

        let closed = registry
            .close_db(Some("default"))
            .expect("close should succeed");
        assert!(closed.closed);
        assert_eq!(closed.db_id, "default");
    }
}
