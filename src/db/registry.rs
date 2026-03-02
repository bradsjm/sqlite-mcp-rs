use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::DEFAULT_DB_ID;
use crate::contracts::db::{DbListData, DbMode, DbOpenData, DbSummary, ExtensionsLoaded};
use crate::errors::{AppError, AppResult};

use super::persistence::{list_persisted_entries, resolve_persist_path};

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
    ) -> AppResult<DbOpenData> {
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
                    extensions_loaded: ExtensionsLoaded {
                        vec: false,
                        rembed: false,
                        regex: false,
                    },
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
            extensions_loaded: ExtensionsLoaded {
                vec: false,
                rembed: false,
                regex: false,
            },
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::contracts::db::DbMode;

    use super::DbRegistry;

    #[test]
    fn opens_memory_db() {
        let mut registry = DbRegistry::default();
        let opened = registry
            .open_db("default".to_string(), DbMode::Memory, None, false, None)
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
        );
        assert!(result.is_ok());
    }
}
