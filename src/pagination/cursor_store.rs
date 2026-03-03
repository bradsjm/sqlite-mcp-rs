//! Cursor store for paginated query results.
//!
//! Provides storage and management of cursor states for resuming
//! large query results across multiple requests.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::contracts::sql::SqlParams;
use uuid::Uuid;

/// State stored for a query cursor.
///
/// Captures all information needed to resume a paginated query,
/// including the SQL, parameters, and current offset.
#[derive(Debug, Clone)]
pub struct CursorState {
    /// Database identifier for the query.
    pub db_id: String,
    /// Query fingerprint for validation.
    pub fingerprint: String,
    /// Current row offset for pagination.
    pub offset: usize,
    /// SQL query string.
    pub sql: String,
    /// Bound parameters for the query.
    pub params: Option<SqlParams>,
    /// Maximum rows per page.
    pub max_rows: usize,
    /// Maximum bytes per page.
    pub max_bytes: usize,
}

/// Internal entry storing cursor state with expiration.
#[derive(Debug, Clone)]
struct CursorEntry {
    state: CursorState,
    expires_at: Instant,
}

/// Store for managing query cursors with TTL and capacity limits.
///
/// Provides thread-safe storage for cursor states with automatic expiration
/// and LRU-style eviction when capacity is exceeded.
#[derive(Debug)]
pub struct CursorStore {
    /// Time-to-live for cursor entries.
    ttl: Duration,
    /// Maximum number of cursors to store.
    capacity: usize,
    /// Map of cursor IDs to their entries.
    entries: HashMap<String, CursorEntry>,
}

impl CursorStore {
    /// Creates a new cursor store with the specified TTL and capacity.
    ///
    /// # Arguments
    ///
    /// * `ttl` - Time-to-live for cursor entries
    /// * `capacity` - Maximum number of cursors to store (0 disables cursors)
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            entries: HashMap::new(),
        }
    }

    /// Creates a new cursor with the given state.
    ///
    /// Returns the cursor ID if cursors are enabled and there's capacity,
    /// otherwise returns None.
    ///
    /// Automatically evicts expired entries and oldest entries if at capacity.
    pub fn create(&mut self, state: CursorState) -> Option<String> {
        if !self.enabled() {
            return None;
        }
        self.evict_expired();
        self.evict_to_capacity();

        let cursor = Uuid::new_v4().to_string();
        let entry = CursorEntry {
            state,
            expires_at: Instant::now() + self.ttl,
        };
        self.entries.insert(cursor.clone(), entry);
        Some(cursor)
    }

    /// Returns whether cursor storage is enabled (capacity > 0).
    pub fn enabled(&self) -> bool {
        self.capacity > 0
    }

    /// Retrieves the state for a cursor and refreshes its TTL.
    ///
    /// Returns None if the cursor doesn't exist or has expired.
    pub fn get(&mut self, cursor: &str) -> Option<CursorState> {
        self.evict_expired();
        let entry = self.entries.get_mut(cursor)?;
        entry.expires_at = Instant::now() + self.ttl;
        Some(entry.state.clone())
    }

    /// Updates the offset for an existing cursor.
    ///
    /// Returns true if the cursor was found and updated, false otherwise.
    pub fn update_offset(&mut self, cursor: &str, offset: usize) -> bool {
        self.evict_expired();
        if let Some(entry) = self.entries.get_mut(cursor) {
            entry.state.offset = offset;
            entry.expires_at = Instant::now() + self.ttl;
            return true;
        }
        false
    }

    /// Deletes a cursor from the store.
    ///
    /// Returns true if the cursor existed and was deleted.
    pub fn delete(&mut self, cursor: &str) -> bool {
        self.entries.remove(cursor).is_some()
    }

    /// Invalidates all cursors for a specific database.
    ///
    /// Called when a database is closed to prevent stale cursor usage.
    pub fn invalidate_db(&mut self, db_id: &str) {
        self.entries.retain(|_, entry| entry.state.db_id != db_id);
    }

    fn evict_expired(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, entry| entry.expires_at > now);
    }

    fn evict_to_capacity(&mut self) {
        if self.capacity == 0 {
            self.entries.clear();
            return;
        }

        while self.entries.len() >= self.capacity {
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest_key);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::{CursorState, CursorStore};

    fn sample_state(offset: usize) -> CursorState {
        CursorState {
            db_id: "default".to_string(),
            fingerprint: "select-1".to_string(),
            offset,
            sql: "select 1".to_string(),
            params: None,
            max_rows: 100,
            max_bytes: 10_000,
        }
    }

    #[test]
    fn creates_and_gets_cursor() {
        let mut store = CursorStore::new(Duration::from_secs(60), 10);
        let cursor = store
            .create(sample_state(0))
            .expect("cursor creation should be enabled");
        let state = store.get(&cursor).expect("cursor should exist");
        assert_eq!(state.offset, 0);
    }

    #[test]
    fn updates_offset() {
        let mut store = CursorStore::new(Duration::from_secs(60), 10);
        let cursor = store
            .create(sample_state(0))
            .expect("cursor creation should be enabled");
        assert!(store.update_offset(&cursor, 12));
        let state = store.get(&cursor).expect("cursor should exist");
        assert_eq!(state.offset, 12);
    }

    #[test]
    fn expires_entries() {
        let mut store = CursorStore::new(Duration::from_millis(5), 10);
        let cursor = store
            .create(sample_state(0))
            .expect("cursor creation should be enabled");
        thread::sleep(Duration::from_millis(8));
        assert!(store.get(&cursor).is_none());
    }

    #[test]
    fn invalidates_db_entries() {
        let mut store = CursorStore::new(Duration::from_secs(60), 10);
        let cursor = store
            .create(sample_state(1))
            .expect("cursor creation should be enabled");
        store.invalidate_db("default");
        assert!(store.get(&cursor).is_none());
    }

    #[test]
    fn capacity_zero_disables_cursors() {
        let mut store = CursorStore::new(Duration::from_secs(60), 0);
        assert!(!store.enabled());
        assert!(store.create(sample_state(0)).is_none());
    }
}
