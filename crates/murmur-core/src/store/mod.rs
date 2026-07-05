//! Single-writer storage API (spec §9). ALL mutations flow through `Store`
//! methods so a change-log/CRDT layer can be inserted later without touching
//! callers. Rows carry created_at/updated_at/device_id; deletes are tombstones.

pub(crate) mod migrations;

mod artifacts;
mod contacts;
mod documents;
mod items;
mod jobs;
mod sessions;
mod usage;

use std::path::Path;
use std::sync::Arc;

use harness::Clock;
use rusqlite::Connection;

use crate::error::CoreError;

// epoch-seconds
fn system_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) device_id: String,
    clock: Clock,
}

impl Store {
    pub fn open(path: impl AsRef<Path>, device_id: impl Into<String>) -> Result<Self, CoreError> {
        Self::from_connection(Connection::open(path)?, device_id)
    }

    pub fn open_in_memory(device_id: impl Into<String>) -> Result<Self, CoreError> {
        Self::from_connection(Connection::open_in_memory()?, device_id)
    }

    fn from_connection(conn: Connection, device_id: impl Into<String>) -> Result<Self, CoreError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        migrations::migrate(&conn)?;
        Ok(Store { conn, device_id: device_id.into(), clock: Arc::new(system_clock) })
    }

    /// Replaces the clock (tests inject deterministic time).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub(crate) fn now(&self) -> u64 {
        (self.clock)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_migrates_to_latest() {
        let store = Store::open_in_memory("device-a").unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, migrations::MIGRATIONS.len());
    }

    #[test]
    fn reopen_is_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "murmur-core-test-{}",
            crate::ids::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("murmur.db");
        {
            Store::open(&path, "device-a").unwrap();
        }
        let store = Store::open(&path, "device-a").unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, migrations::MIGRATIONS.len());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn injected_clock_drives_now() {
        let store = Store::open_in_memory("device-a")
            .unwrap()
            .with_clock(std::sync::Arc::new(|| 12345));
        assert_eq!(store.now(), 12345);
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let store = Store::open_in_memory("device-a").unwrap();
        let result = store.conn.execute(
            "INSERT INTO sessions (id, status, transcript, started_at, created_at, updated_at, device_id)
             VALUES ('s1', 'recording', '', 1, 1, 1, 'd')",
            [],
        );
        assert!(result.is_ok(), "null job_id is fine");
        let result = store.conn.execute(
            "INSERT INTO sessions (id, job_id, status, transcript, started_at, created_at, updated_at, device_id)
             VALUES ('s2', 'no-such-job', 'recording', '', 1, 1, 1, 'd')",
            [],
        );
        assert!(result.is_err(), "dangling job_id must be rejected");
    }
}
