use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::contracts::sql::SqlParams;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CursorState {
    pub db_id: String,
    pub fingerprint: String,
    pub offset: usize,
    pub sql: String,
    pub params: Option<SqlParams>,
    pub max_rows: usize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone)]
struct CursorEntry {
    state: CursorState,
    expires_at: Instant,
}

#[derive(Debug)]
pub struct CursorStore {
    ttl: Duration,
    capacity: usize,
    entries: HashMap<String, CursorEntry>,
}

impl CursorStore {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            entries: HashMap::new(),
        }
    }

    pub fn create(&mut self, state: CursorState) -> String {
        self.evict_expired();
        self.evict_to_capacity();

        let cursor = Uuid::new_v4().to_string();
        let entry = CursorEntry {
            state,
            expires_at: Instant::now() + self.ttl,
        };
        self.entries.insert(cursor.clone(), entry);
        cursor
    }

    pub fn get(&mut self, cursor: &str) -> Option<CursorState> {
        self.evict_expired();
        let entry = self.entries.get_mut(cursor)?;
        entry.expires_at = Instant::now() + self.ttl;
        Some(entry.state.clone())
    }

    pub fn update_offset(&mut self, cursor: &str, offset: usize) -> bool {
        self.evict_expired();
        if let Some(entry) = self.entries.get_mut(cursor) {
            entry.state.offset = offset;
            entry.expires_at = Instant::now() + self.ttl;
            return true;
        }
        false
    }

    pub fn delete(&mut self, cursor: &str) -> bool {
        self.entries.remove(cursor).is_some()
    }

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
        let cursor = store.create(sample_state(0));
        let state = store.get(&cursor).expect("cursor should exist");
        assert_eq!(state.offset, 0);
    }

    #[test]
    fn updates_offset() {
        let mut store = CursorStore::new(Duration::from_secs(60), 10);
        let cursor = store.create(sample_state(0));
        assert!(store.update_offset(&cursor, 12));
        let state = store.get(&cursor).expect("cursor should exist");
        assert_eq!(state.offset, 12);
    }

    #[test]
    fn expires_entries() {
        let mut store = CursorStore::new(Duration::from_millis(5), 10);
        let cursor = store.create(sample_state(0));
        thread::sleep(Duration::from_millis(8));
        assert!(store.get(&cursor).is_none());
    }

    #[test]
    fn invalidates_db_entries() {
        let mut store = CursorStore::new(Duration::from_secs(60), 10);
        let cursor = store.create(sample_state(1));
        store.invalidate_db("default");
        assert!(store.get(&cursor).is_none());
    }
}
