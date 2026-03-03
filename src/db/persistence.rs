//! Database persistence utilities.
//!
//! Provides path resolution, persisted database listing, and size limit enforcement.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::errors::AppError;

/// Resolves a requested persistence path within the persistence root.
///
/// Validates that the path:
/// - Is not empty
/// - Does not contain parent directory references ("..")
/// - Remains within the persistence root directory
///
/// # Arguments
///
/// * `persist_root` - Root directory for persisted databases
/// * `requested_path` - Requested path (relative or absolute)
///
/// # Errors
///
/// Returns [`AppError::InvalidInput`] for invalid paths.
pub fn resolve_persist_path(
    persist_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, AppError> {
    let requested = PathBuf::from(requested_path.trim());
    if requested.as_os_str().is_empty() {
        return Err(AppError::InvalidInput("path must not be empty".to_string()));
    }

    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AppError::InvalidInput(
            "persisted paths must not contain '..'".to_string(),
        ));
    }

    let canonical_root = persist_root
        .canonicalize()
        .unwrap_or_else(|_| persist_root.to_path_buf());

    let candidate = if requested.is_absolute() {
        requested
    } else {
        canonical_root.join(requested)
    };

    let canonical_candidate = if candidate.exists() {
        candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf())
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| AppError::InvalidInput("path must include a file name".to_string()))?;
        let canonical_parent = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        let file_name = candidate
            .file_name()
            .ok_or_else(|| AppError::InvalidInput("path must include a file name".to_string()))?;
        canonical_parent.join(file_name)
    };

    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(AppError::InvalidInput(
            "persisted path must remain inside SQLITE_PERSIST_ROOT".to_string(),
        ));
    }

    Ok(canonical_candidate)
}

/// Lists persisted database files in the persistence root.
///
/// Recursively scans the persistence directory and returns relative paths
/// to all database files found, sorted alphabetically.
///
/// # Arguments
///
/// * `persist_root` - Root directory to scan
/// * `limit` - Maximum number of entries to return
///
/// # Returns
///
/// Tuple of (entries, truncated) where entries are relative paths and
/// truncated indicates if more entries exist beyond the limit.
pub fn list_persisted_entries(
    persist_root: &Path,
    limit: usize,
) -> Result<(Vec<String>, bool), AppError> {
    if limit == 0 || !persist_root.exists() {
        return Ok((Vec::new(), false));
    }

    let canonical_root = persist_root
        .canonicalize()
        .unwrap_or_else(|_| persist_root.to_path_buf());

    let mut stack = vec![canonical_root.clone()];
    let mut entries = Vec::new();
    let mut truncated = false;

    while let Some(directory) = stack.pop() {
        let read_dir = fs::read_dir(&directory).map_err(|error| {
            AppError::Dependency(format!("failed reading persisted dir: {error}"))
        })?;
        for item in read_dir {
            let item = item.map_err(|error| {
                AppError::Dependency(format!("failed reading persisted entry: {error}"))
            })?;
            let path = item.path();
            let metadata = item.metadata().map_err(|error| {
                AppError::Dependency(format!("failed reading metadata: {error}"))
            })?;

            if metadata.is_dir() {
                stack.push(path);
                continue;
            }

            if !metadata.is_file() {
                continue;
            }

            let Ok(relative) = path.strip_prefix(&canonical_root) else {
                continue;
            };
            entries.push(relative.to_string_lossy().to_string());

            if entries.len() > limit {
                entries.truncate(limit);
                truncated = true;
                break;
            }
        }

        if truncated {
            break;
        }
    }

    entries.sort();
    Ok((entries, truncated))
}

/// Enforces the maximum database file size limit.
///
/// Checks the size of the database file and returns an error if it exceeds
/// the configured limit. This is called after write operations to prevent
/// databases from growing too large.
///
/// # Arguments
///
/// * `path` - Optional path to the database file (None for in-memory databases)
/// * `max_db_bytes` - Maximum allowed file size in bytes
///
/// # Errors
///
/// Returns [`AppError::LimitExceeded`] if the database exceeds the size limit.
pub fn enforce_db_size_limit(path: Option<&Path>, max_db_bytes: u64) -> Result<(), AppError> {
    let Some(path) = path else {
        return Ok(());
    };

    let metadata = fs::metadata(path).map_err(|error| {
        AppError::Dependency(format!("failed to read database file metadata: {error}"))
    })?;
    let size = metadata.len();
    if size > max_db_bytes {
        return Err(AppError::LimitExceeded(format!(
            "database file exceeds SQLITE_MAX_DB_BYTES ({max_db_bytes})"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::db::persistence::list_persisted_entries;

    use super::resolve_persist_path;

    #[test]
    fn keeps_relative_path_inside_root() {
        let root = Path::new("/tmp/sqlite-mcp");
        let resolved = resolve_persist_path(root, "workspace/a.db").expect("path should resolve");
        assert!(resolved.starts_with(root));
    }

    #[test]
    fn rejects_parent_path_segments() {
        let root = Path::new("/tmp/sqlite-mcp");
        let err = resolve_persist_path(root, "../oops.db").expect_err("path must fail");
        assert!(err.to_string().contains("must not contain '..'"));
    }

    #[test]
    fn lists_persisted_entries() {
        let root = std::env::temp_dir().join(format!("sqlite-mcp-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).expect("create nested dir");
        std::fs::write(root.join("a.db"), b"a").expect("write a.db");
        std::fs::write(root.join("nested").join("b.db"), b"b").expect("write b.db");

        let (entries, truncated) = list_persisted_entries(&root, 10).expect("list entries");
        assert_eq!(entries, vec!["a.db".to_string(), "nested/b.db".to_string()]);
        assert!(!truncated);

        let _ = std::fs::remove_dir_all(&root);
    }
}
