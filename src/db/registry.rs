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

#[derive(Debug)]
struct DbHandle {
    connection: Connection,
    mode: DbMode,
    path: Option<PathBuf>,
}

#[derive(Debug)]
pub struct DbRegistry {
    active_db_id: String,
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

        self.handles.remove(&db_id);

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

    pub fn get_connection(&self, db_id: Option<&str>) -> AppResult<&Connection> {
        let resolved_db_id = db_id.unwrap_or(&self.active_db_id);
        self.handles
            .get(resolved_db_id)
            .map(|handle| &handle.connection)
            .ok_or_else(|| AppError::NotFound(format!("unknown db_id: {resolved_db_id}")))
    }

    pub fn persisted_path(&self, db_id: Option<&str>) -> AppResult<Option<PathBuf>> {
        let resolved_db_id = db_id.unwrap_or(&self.active_db_id);
        let handle = self
            .handles
            .get(resolved_db_id)
            .ok_or_else(|| AppError::NotFound(format!("unknown db_id: {resolved_db_id}")))?;
        Ok(handle.path.clone())
    }
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

    use super::DbRegistry;

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
