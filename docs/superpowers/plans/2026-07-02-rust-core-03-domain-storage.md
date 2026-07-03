# Murmur Rust Core — Plan 03: Domain + Sync-Ready Storage

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `murmur-core` crate: domain entities (Job, Session, CapturedItem, Contact, Artifact), a sync-ready SQLite store behind a single-writer API, session lifecycle, the session library (list + text search — the episodic memory tier), and persistence/derivation of the harness's reflection signals.

**Architecture:** New crate `crates/murmur-core` at `/Users/claude/murmur-rmp`, first consumer of `harness`. One `Store` struct owns a rusqlite `Connection`; ALL mutations go through its methods (spec §9: single writer API so a change-log/CRDT layer can be inserted later). Every row carries `created_at`, `updated_at`, `device_id`; deletes are tombstones (`deleted_at`); ids are UUIDv7 strings. Migrations run via `PRAGMA user_version`. No LLM calls in this plan — everything is hermetic. The end-of-session processing pipeline (agent tools `create_report`/`update_todos`/`upsert_contact`, reflection coordinator with pre-reflection snapshot) is **Plan 04**; live extraction 05; STT 06; layout protocol + FFI 07 (series resequenced — processing pipeline was implicit in old 03).

**Tech Stack:** rusqlite (bundled SQLite — NEW workspace dep), uuid v7 (NEW workspace dep), serde/serde_json/thiserror (existing), harness (path dep — reuses `Clock` and `ReflectionSignals`). Timestamps are unix seconds (`u64` in domain, `INTEGER` in SQLite); tests inject clocks.

**Spec:** `docs/superpowers/specs/2026-07-01-murmur-rebuild-vision-design.md` §9 (storage & sync-readiness), §2 stories 7/9/10 (customer recall, session library, manual parity), Rev 2 §1/§3 (artifact seam, Job first-class), §7 (reflection signals live in the app layer). Carried from Plan 02's final review: `ReflectionSignals::record_reflection` is the consumption path; reflection-time snapshotting and NFC normalization are Plan 04 concerns (noted in §Deferred at the bottom).

---

## File Structure

```
Cargo.toml                      # MODIFY: add member, rusqlite + uuid workspace deps
crates/murmur-core/
  Cargo.toml                    # NEW
  src/
    lib.rs                      # module wiring + re-exports
    error.rs                    # CoreError
    ids.rs                      # new_id() — UUIDv7 strings
    domain.rs                   # Job/Session/CapturedItem/Contact/Artifact + status enums
    store/
      mod.rs                    # Store: open/open_in_memory/with_clock, now(), device_id
      migrations.rs             # MIGRATIONS + migrate()
      jobs.rs                   # impl Store: job CRUD
      sessions.rs               # impl Store: session lifecycle + library + search
      items.rs                  # impl Store: captured items
      contacts.rs               # impl Store: contact upsert/CRUD
      artifacts.rs              # impl Store: artifact seam CRUD
    reflection.rs               # impl Store: signals persistence + activity derivation
```

Run cargo on this host via the repo dev shell (`direnv`/`nix develop`) or `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo <cmd>`. First build after adding rusqlite compiles bundled SQLite — expect a few minutes.

---

### Task 1: Crate scaffold, CoreError, UUIDv7 ids

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/murmur-core/Cargo.toml`, `crates/murmur-core/src/lib.rs`, `src/error.rs`, `src/ids.rs`

- [ ] **Step 1: Wire the crate**

Workspace root `Cargo.toml`: members becomes `["crates/harness", "crates/murmur-core"]`; add to `[workspace.dependencies]`:
```toml
rusqlite = { version = "0.32", features = ["bundled"] }
uuid = { version = "1", features = ["v7"] }
```

`crates/murmur-core/Cargo.toml`:
```toml
[package]
name = "murmur-core"
version = "0.1.0"
edition = "2021"

[dependencies]
harness = { path = "../harness" }
rusqlite = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true }
```

- [ ] **Step 2: Write the failing test** (bottom of `src/ids.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_is_a_uuid_v7() {
        let id = new_id();
        let parsed = uuid::Uuid::parse_str(&id).expect("valid uuid");
        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn ids_are_unique_and_time_ordered() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert!(a <= b, "uuid v7 string order follows creation order");
    }
}
```

- [ ] **Step 3: Run to see failure**

Run: `cargo test -p murmur-core`
Expected: compile FAIL (`new_id` not found).

- [ ] **Step 4: Implement**

`src/ids.rs` (above tests):
```rust
//! UUIDv7 ids (spec §9): time-ordered, sync-safe, sortable as strings.

/// A fresh UUIDv7 as a lowercase hyphenated string.
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
```

`src/error.rs`:
```rust
use thiserror::Error;

/// Errors from the murmur-core domain layer.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("corrupt row: {0}")]
    Corrupt(String),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid state: {0}")]
    InvalidState(String),
}
```

`src/lib.rs`:
```rust
pub mod error;
pub mod ids;

pub use error::CoreError;
pub use ids::new_id;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p murmur-core`
Expected: 2 pass. Also run `cargo test` — harness's 64 still pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(core): murmur-core crate scaffold with CoreError and UUIDv7 ids"
```

---

### Task 2: Store::open + migrations

**Files:**
- Create: `src/store/mod.rs`, `src/store/migrations.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/store/mod.rs`)

```rust
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
        );
        assert!(result.is_ok(), "null job_id is fine");
        let result = store.conn.execute(
            "INSERT INTO sessions (id, job_id, status, transcript, started_at, created_at, updated_at, device_id)
             VALUES ('s2', 'no-such-job', 'recording', '', 1, 1, 1, 'd')",
        );
        assert!(result.is_err(), "dangling job_id must be rejected");
    }
}
```

Note: `conn.execute` in rusqlite takes `(sql, params)`. Use `store.conn.execute(SQL, [])` — adapt the test calls to include the empty params array.

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core store`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/store/migrations.rs`:
```rust
use rusqlite::Connection;

use crate::error::CoreError;

/// One entry per schema version, applied in order. NEVER edit an existing
/// entry after it has shipped — append a new one.
pub(crate) const MIGRATIONS: &[&str] = &[
    // v1: initial schema (spec §9: timestamps + device id on every row, tombstones)
    r#"
    CREATE TABLE jobs (
        id           TEXT PRIMARY KEY,
        name         TEXT NOT NULL,
        client       TEXT,
        site         TEXT,
        scheduled_at INTEGER,
        status       TEXT NOT NULL,
        created_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL,
        device_id    TEXT NOT NULL,
        deleted_at   INTEGER
    );

    CREATE TABLE sessions (
        id         TEXT PRIMARY KEY,
        job_id     TEXT REFERENCES jobs(id),
        status     TEXT NOT NULL,
        transcript TEXT NOT NULL DEFAULT '',
        summary    TEXT,
        started_at INTEGER NOT NULL,
        ended_at   INTEGER,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        device_id  TEXT NOT NULL,
        deleted_at INTEGER
    );

    CREATE TABLE items (
        id         TEXT PRIMARY KEY,
        session_id TEXT NOT NULL REFERENCES sessions(id),
        kind       TEXT NOT NULL,
        text       TEXT NOT NULL,
        done       INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        device_id  TEXT NOT NULL,
        deleted_at INTEGER
    );

    CREATE TABLE contacts (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        trade      TEXT,
        phone      TEXT,
        notes      TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        device_id  TEXT NOT NULL,
        deleted_at INTEGER
    );

    CREATE TABLE artifacts (
        id         TEXT PRIMARY KEY,
        session_id TEXT NOT NULL REFERENCES sessions(id),
        kind       TEXT NOT NULL,
        title      TEXT NOT NULL,
        body       TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        device_id  TEXT NOT NULL,
        deleted_at INTEGER
    );

    CREATE TABLE reflection_state (
        id                INTEGER PRIMARY KEY CHECK (id = 1),
        signals           TEXT NOT NULL,
        last_reflected_at INTEGER NOT NULL DEFAULT 0
    );

    CREATE INDEX idx_sessions_started ON sessions(started_at);
    CREATE INDEX idx_items_session ON items(session_id);
    CREATE INDEX idx_artifacts_session ON artifacts(session_id);
    "#,
];

pub(crate) fn migrate(conn: &Connection) -> Result<(), CoreError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    Ok(())
}
```

`src/store/mod.rs` (above tests):
```rust
//! Single-writer storage API (spec §9). ALL mutations flow through `Store`
//! methods so a change-log/CRDT layer can be inserted later without touching
//! callers. Rows carry created_at/updated_at/device_id; deletes are tombstones.

pub(crate) mod migrations;

mod artifacts;
mod contacts;
mod items;
mod jobs;
mod sessions;

use std::path::Path;
use std::sync::Arc;

use harness::Clock;
use rusqlite::Connection;

use crate::error::CoreError;

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
```

The `mod artifacts; mod contacts; mod items; mod jobs; mod sessions;` lines refer to files created in Tasks 3–6 — create them now as EMPTY files (`touch`), so this task compiles standalone.

`src/lib.rs` — add:
```rust
pub mod domain;
pub mod store;

pub use store::Store;
```
(`pub mod domain;` lands in Task 3 — if you prefer strict sequencing, add it in Task 3 instead. Do NOT add `pub mod reflection;` yet.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`
Expected: Task 1's 2 + these 4 pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): sqlite Store with user_version migrations, tombstone-ready schema"
```

---

### Task 3: Domain types + Job CRUD

**Files:**
- Create: `src/domain.rs`, fill `src/store/jobs.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/store/jobs.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::domain::{JobStatus, NewJob};
    use crate::error::CoreError;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000))
    }

    fn new_job(name: &str) -> NewJob {
        NewJob {
            name: name.into(),
            client: Some("Johnson".into()),
            site: Some("14 Elm St".into()),
            scheduled_at: Some(2000),
        }
    }

    #[test]
    fn create_and_get_round_trip() {
        let s = store();
        let job = s.create_job(new_job("Johnson remodel")).unwrap();
        assert_eq!(job.status, JobStatus::Active);
        assert_eq!(job.created_at, 1000);
        assert_eq!(job.updated_at, 1000);
        assert_eq!(job.device_id, "device-a");
        let got = s.get_job(&job.id).unwrap();
        assert_eq!(got, job);
    }

    #[test]
    fn get_missing_is_not_found() {
        let s = store();
        assert!(matches!(
            s.get_job("nope"),
            Err(CoreError::NotFound { entity: "job", .. })
        ));
    }

    #[test]
    fn list_orders_scheduled_first_then_recent() {
        let s = store();
        let unscheduled = s
            .create_job(NewJob { name: "backlog".into(), client: None, site: None, scheduled_at: None })
            .unwrap();
        let later = s.create_job(NewJob { scheduled_at: Some(900), ..new_job("later") }).unwrap();
        let sooner = s.create_job(NewJob { scheduled_at: Some(800), ..new_job("sooner") }).unwrap();
        let ids: Vec<_> = s.list_jobs().unwrap().into_iter().map(|j| j.id).collect();
        assert_eq!(ids, vec![sooner.id, later.id, unscheduled.id]);
    }

    #[test]
    fn update_status_touches_updated_at() {
        let s = store();
        let job = s.create_job(new_job("j")).unwrap();
        let s = s.with_clock(Arc::new(|| 1500));
        let done = s.update_job_status(&job.id, JobStatus::Done).unwrap();
        assert_eq!(done.status, JobStatus::Done);
        assert_eq!(done.updated_at, 1500);
        assert_eq!(done.created_at, 1000, "created_at never changes");
    }

    #[test]
    fn delete_is_a_tombstone() {
        let s = store();
        let job = s.create_job(new_job("gone")).unwrap();
        s.delete_job(&job.id).unwrap();
        assert!(matches!(s.get_job(&job.id), Err(CoreError::NotFound { .. })));
        assert!(s.list_jobs().unwrap().is_empty());
        // the row still exists (tombstone, not erase)
        let raw: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM jobs WHERE id = ?1", [&job.id], |r| r.get(0))
            .unwrap();
        assert_eq!(raw, 1);
        // deleting again is NotFound
        assert!(matches!(s.delete_job(&job.id), Err(CoreError::NotFound { .. })));
    }

    #[test]
    fn unknown_status_string_is_corrupt() {
        assert!(matches!(JobStatus::parse("garbage"), Err(CoreError::Corrupt(_))));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core jobs`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/domain.rs`:
```rust
//! Domain entities (spec §2, Rev 2 §3: Job is first-class; artifacts are a seam).
//! Plain serde data — these types cross the FFI boundary in Plan 07.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Done,
    Archived,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Active => "active",
            JobStatus::Done => "done",
            JobStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "active" => Ok(JobStatus::Active),
            "done" => Ok(JobStatus::Done),
            "archived" => Ok(JobStatus::Archived),
            other => Err(CoreError::Corrupt(format!("unknown job status: {other}"))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Audio/transcript still coming in.
    Recording,
    /// Ended; queued for the processing pipeline (Plan 04). Offline-safe.
    AwaitingProcessing,
    /// Pipeline finished; summary and artifacts exist.
    Processed,
    /// Pipeline failed; retryable.
    Failed,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Recording => "recording",
            SessionStatus::AwaitingProcessing => "awaiting_processing",
            SessionStatus::Processed => "processed",
            SessionStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "recording" => Ok(SessionStatus::Recording),
            "awaiting_processing" => Ok(SessionStatus::AwaitingProcessing),
            "processed" => Ok(SessionStatus::Processed),
            "failed" => Ok(SessionStatus::Failed),
            other => Err(CoreError::Corrupt(format!("unknown session status: {other}"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub client: Option<String>,
    pub site: Option<String>,
    /// Unix seconds; None = unscheduled/backlog.
    pub scheduled_at: Option<u64>,
    pub status: JobStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct NewJob {
    pub name: String,
    pub client: Option<String>,
    pub site: Option<String>,
    pub scheduled_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub job_id: Option<String>,
    pub status: SessionStatus,
    pub transcript: String,
    /// Filled by the processing pipeline (Plan 04); also feeds reflection activity.
    pub summary: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

/// A typed item extracted from (or manually added to) a session.
/// `kind` is a free string by design — conventions: "todo", "decision",
/// "note", "safety", "part", "price". New kinds must not require a migration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapturedItem {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub text: String,
    pub done: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub trade: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

/// The artifact seam (Rev 2 §1): generated documents of any kind hang off a
/// session. `kind` is a free string ("report", "estimate", …); generators
/// register in Plan 04. `body` is markdown (or JSON for structured kinds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}
```

`src/store/jobs.rs` (above tests):
```rust
use rusqlite::Row;

use crate::domain::{Job, JobStatus, NewJob};
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

fn job_from_row(row: &Row) -> Result<Job, rusqlite::Error> {
    Ok(Job {
        id: row.get("id")?,
        name: row.get("name")?,
        client: row.get("client")?,
        site: row.get("site")?,
        scheduled_at: row.get::<_, Option<i64>>("scheduled_at")?.map(|v| v as u64),
        // status parsed by the caller — see note below
        status: JobStatus::Active,
        created_at: row.get::<_, i64>("created_at")? as u64,
        updated_at: row.get::<_, i64>("updated_at")? as u64,
        device_id: row.get("device_id")?,
    })
}

fn job_from_row_full(row: &Row) -> Result<Job, CoreError> {
    let status_raw: String = row.get("status").map_err(CoreError::Sqlite)?;
    let mut job = job_from_row(row).map_err(CoreError::Sqlite)?;
    job.status = JobStatus::parse(&status_raw)?;
    Ok(job)
}

const JOB_COLS: &str = "id, name, client, site, scheduled_at, status, created_at, updated_at, device_id";

impl Store {
    pub fn create_job(&self, new: NewJob) -> Result<Job, CoreError> {
        let now = self.now();
        let job = Job {
            id: new_id(),
            name: new.name,
            client: new.client,
            site: new.site,
            scheduled_at: new.scheduled_at,
            status: JobStatus::Active,
            created_at: now,
            updated_at: now,
            device_id: self.device_id.clone(),
        };
        self.conn.execute(
            "INSERT INTO jobs (id, name, client, site, scheduled_at, status, created_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                job.id,
                job.name,
                job.client,
                job.site,
                job.scheduled_at.map(|v| v as i64),
                job.status.as_str(),
                job.created_at as i64,
                job.updated_at as i64,
                job.device_id,
            ],
        )?;
        Ok(job)
    }

    pub fn get_job(&self, id: &str) -> Result<Job, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {JOB_COLS} FROM jobs WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => job_from_row_full(row),
            None => Err(CoreError::NotFound { entity: "job", id: id.to_string() }),
        }
    }

    /// Jobs board order: scheduled first (soonest on top), then unscheduled by
    /// recency. Tombstoned rows excluded.
    pub fn list_jobs(&self) -> Result<Vec<Job>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {JOB_COLS} FROM jobs WHERE deleted_at IS NULL
             ORDER BY scheduled_at IS NULL, scheduled_at ASC, created_at DESC"
        ))?;
        let mut rows = stmt.query([])?;
        let mut jobs = Vec::new();
        while let Some(row) = rows.next()? {
            jobs.push(job_from_row_full(row)?);
        }
        Ok(jobs)
    }

    pub fn update_job_status(&self, id: &str, status: JobStatus) -> Result<Job, CoreError> {
        let changed = self.conn.execute(
            "UPDATE jobs SET status = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            rusqlite::params![status.as_str(), self.now() as i64, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "job", id: id.to_string() });
        }
        self.get_job(id)
    }

    pub fn delete_job(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let changed = self.conn.execute(
            "UPDATE jobs SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "job", id: id.to_string() });
        }
        Ok(())
    }
}
```

Note on `job_from_row`/`job_from_row_full`: the split exists because status parsing returns `CoreError`, not `rusqlite::Error`. If you find a cleaner single-function shape that keeps error types straight, use it — the behavior contract is what the tests pin.

`src/lib.rs` — ensure:
```rust
pub mod domain;
pub use domain::{
    Artifact, CapturedItem, Contact, Job, JobStatus, NewJob, Session, SessionStatus,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): domain types and tombstoning Job CRUD through the writer API"
```

---

### Task 4: Session lifecycle + library

**Files:**
- Fill: `src/store/sessions.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/store/sessions.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::domain::{NewJob, SessionStatus};
    use crate::error::CoreError;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000))
    }

    #[test]
    fn start_append_end_lifecycle() {
        let s = store();
        let session = s.start_session(None).unwrap();
        assert_eq!(session.status, SessionStatus::Recording);
        assert_eq!(session.started_at, 1000);
        assert!(session.ended_at.is_none());

        s.append_transcript(&session.id, "we need to fix the deck. ").unwrap();
        s.append_transcript(&session.id, "call Dev about the framing.").unwrap();

        let s = s.with_clock(Arc::new(|| 2000));
        let ended = s.end_session(&session.id).unwrap();
        assert_eq!(ended.status, SessionStatus::AwaitingProcessing);
        assert_eq!(ended.ended_at, Some(2000));
        assert_eq!(
            ended.transcript,
            "we need to fix the deck. call Dev about the framing."
        );
    }

    #[test]
    fn start_with_job_links_and_validates() {
        let s = store();
        let job = s
            .create_job(NewJob { name: "j".into(), ..Default::default() })
            .unwrap();
        let session = s.start_session(Some(&job.id)).unwrap();
        assert_eq!(session.job_id.as_deref(), Some(job.id.as_str()));
        assert!(matches!(
            s.start_session(Some("no-such-job")),
            Err(CoreError::NotFound { entity: "job", .. })
        ));
    }

    #[test]
    fn end_requires_recording_state() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.end_session(&session.id).unwrap();
        assert!(matches!(
            s.end_session(&session.id),
            Err(CoreError::InvalidState(_))
        ));
    }

    #[test]
    fn append_to_ended_session_is_invalid() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.end_session(&session.id).unwrap();
        assert!(matches!(
            s.append_transcript(&session.id, "late words"),
            Err(CoreError::InvalidState(_))
        ));
    }

    #[test]
    fn mark_processed_sets_summary() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.end_session(&session.id).unwrap();
        let done = s
            .mark_session_processed(&session.id, "Walked the deck; 2 todos.")
            .unwrap();
        assert_eq!(done.status, SessionStatus::Processed);
        assert_eq!(done.summary.as_deref(), Some("Walked the deck; 2 todos."));
    }

    #[test]
    fn mark_failed_is_retryable_state() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.end_session(&session.id).unwrap();
        let failed = s.mark_session_failed(&session.id).unwrap();
        assert_eq!(failed.status, SessionStatus::Failed);
    }

    #[test]
    fn library_lists_reverse_chronological() {
        let s = store().with_clock(Arc::new(|| 100));
        let a = s.start_session(None).unwrap();
        let s = s.with_clock(Arc::new(|| 200));
        let b = s.start_session(None).unwrap();
        let ids: Vec<_> = s.list_sessions().unwrap().into_iter().map(|x| x.id).collect();
        assert_eq!(ids, vec![b.id, a.id]);
    }

    #[test]
    fn delete_session_is_a_tombstone() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.delete_session(&session.id).unwrap();
        assert!(matches!(s.get_session(&session.id), Err(CoreError::NotFound { .. })));
        assert!(s.list_sessions().unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core sessions`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `src/store/sessions.rs`)

```rust
use rusqlite::Row;

use crate::domain::{Session, SessionStatus};
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

const SESSION_COLS: &str =
    "id, job_id, status, transcript, summary, started_at, ended_at, created_at, updated_at, device_id";

fn session_from_row(row: &Row) -> Result<Session, CoreError> {
    let status_raw: String = row.get("status").map_err(CoreError::Sqlite)?;
    Ok(Session {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        job_id: row.get("job_id").map_err(CoreError::Sqlite)?,
        status: SessionStatus::parse(&status_raw)?,
        transcript: row.get("transcript").map_err(CoreError::Sqlite)?,
        summary: row.get("summary").map_err(CoreError::Sqlite)?,
        started_at: row.get::<_, i64>("started_at").map_err(CoreError::Sqlite)? as u64,
        ended_at: row
            .get::<_, Option<i64>>("ended_at")
            .map_err(CoreError::Sqlite)?
            .map(|v| v as u64),
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        updated_at: row.get::<_, i64>("updated_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
    })
}

impl Store {
    /// Starts a recording session. `job_id` is optional (R4: no pre-labeling
    /// required — the pipeline can link a job later from content).
    pub fn start_session(&self, job_id: Option<&str>) -> Result<Session, CoreError> {
        if let Some(jid) = job_id {
            self.get_job(jid)?; // validates existence + not tombstoned
        }
        let now = self.now();
        let session = Session {
            id: new_id(),
            job_id: job_id.map(str::to_string),
            status: SessionStatus::Recording,
            transcript: String::new(),
            summary: None,
            started_at: now,
            ended_at: None,
            created_at: now,
            updated_at: now,
            device_id: self.device_id.clone(),
        };
        self.conn.execute(
            "INSERT INTO sessions (id, job_id, status, transcript, started_at, created_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                session.id,
                session.job_id,
                session.status.as_str(),
                session.transcript,
                session.started_at as i64,
                session.created_at as i64,
                session.updated_at as i64,
                session.device_id,
            ],
        )?;
        Ok(session)
    }

    /// Appends a transcript chunk. Transcript persists continuously (spec §6:
    /// a dead battery loses nothing) — call this per STT segment.
    pub fn append_transcript(&self, id: &str, chunk: &str) -> Result<(), CoreError> {
        let session = self.get_session(id)?;
        if session.status != SessionStatus::Recording {
            return Err(CoreError::InvalidState(format!(
                "cannot append transcript to a {} session",
                session.status.as_str()
            )));
        }
        self.conn.execute(
            "UPDATE sessions SET transcript = transcript || ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![chunk, self.now() as i64, id],
        )?;
        Ok(())
    }

    /// Ends recording; the session queues for processing (offline-safe).
    pub fn end_session(&self, id: &str) -> Result<Session, CoreError> {
        let session = self.get_session(id)?;
        if session.status != SessionStatus::Recording {
            return Err(CoreError::InvalidState(format!(
                "cannot end a {} session",
                session.status.as_str()
            )));
        }
        let now = self.now();
        self.conn.execute(
            "UPDATE sessions SET status = ?1, ended_at = ?2, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![SessionStatus::AwaitingProcessing.as_str(), now as i64, id],
        )?;
        self.get_session(id)
    }

    /// Pipeline success (Plan 04 calls this). Summary feeds the session
    /// library and reflection activity.
    pub fn mark_session_processed(&self, id: &str, summary: &str) -> Result<Session, CoreError> {
        self.transition_ended(id, SessionStatus::Processed, Some(summary))
    }

    /// Pipeline failure; retryable.
    pub fn mark_session_failed(&self, id: &str) -> Result<Session, CoreError> {
        self.transition_ended(id, SessionStatus::Failed, None)
    }

    fn transition_ended(
        &self,
        id: &str,
        to: SessionStatus,
        summary: Option<&str>,
    ) -> Result<Session, CoreError> {
        let session = self.get_session(id)?;
        if session.status == SessionStatus::Recording {
            return Err(CoreError::InvalidState(
                "session is still recording".to_string(),
            ));
        }
        match summary {
            Some(text) => self.conn.execute(
                "UPDATE sessions SET status = ?1, summary = ?2, updated_at = ?3 WHERE id = ?4",
                rusqlite::params![to.as_str(), text, self.now() as i64, id],
            )?,
            None => self.conn.execute(
                "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![to.as_str(), self.now() as i64, id],
            )?,
        };
        self.get_session(id)
    }

    pub fn get_session(&self, id: &str) -> Result<Session, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => session_from_row(row),
            None => Err(CoreError::NotFound { entity: "session", id: id.to_string() }),
        }
    }

    /// The session library (story 9): reverse-chronological.
    pub fn list_sessions(&self) -> Result<Vec<Session>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE deleted_at IS NULL
             ORDER BY started_at DESC, id DESC"
        ))?;
        let mut rows = stmt.query([])?;
        let mut sessions = Vec::new();
        while let Some(row) = rows.next()? {
            sessions.push(session_from_row(row)?);
        }
        Ok(sessions)
    }

    pub fn delete_session(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let changed = self.conn.execute(
            "UPDATE sessions SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "session", id: id.to_string() });
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): session lifecycle with offline-safe states and reverse-chron library"
```

---

### Task 5: CapturedItems

**Files:**
- Fill: `src/store/items.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/store/items.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::error::CoreError;
    use crate::store::Store;

    fn store_with_session() -> (Store, String) {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let session = s.start_session(None).unwrap();
        (s, session.id)
    }

    #[test]
    fn add_and_list_in_insertion_order() {
        let (s, sid) = store_with_session();
        let a = s.add_item(&sid, "todo", "order lumber").unwrap();
        let b = s.add_item(&sid, "safety", "loose railing on deck").unwrap();
        assert_eq!(a.kind, "todo");
        assert!(!a.done);
        let items = s.list_items_for_session(&sid).unwrap();
        assert_eq!(items, vec![a, b]);
    }

    #[test]
    fn add_to_missing_session_is_not_found() {
        let (s, _) = store_with_session();
        assert!(matches!(
            s.add_item("nope", "todo", "x"),
            Err(CoreError::NotFound { entity: "session", .. })
        ));
    }

    #[test]
    fn done_toggle_round_trips() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "order lumber").unwrap();
        let s = s.with_clock(Arc::new(|| 2000));
        let done = s.set_item_done(&item.id, true).unwrap();
        assert!(done.done);
        assert_eq!(done.updated_at, 2000);
        let undone = s.set_item_done(&item.id, false).unwrap();
        assert!(!undone.done);
    }

    #[test]
    fn open_todos_span_sessions_and_skip_done() {
        let (s, sid_a) = store_with_session();
        let sid_b = s.start_session(None).unwrap().id;
        let t1 = s.add_item(&sid_a, "todo", "one").unwrap();
        let t2 = s.add_item(&sid_b, "todo", "two").unwrap();
        s.add_item(&sid_b, "note", "not a todo").unwrap();
        s.set_item_done(&t1.id, true).unwrap();
        let open: Vec<_> = s.list_open_todos().unwrap().into_iter().map(|i| i.id).collect();
        assert_eq!(open, vec![t2.id]);
    }

    #[test]
    fn delete_item_is_a_tombstone() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "x").unwrap();
        s.delete_item(&item.id).unwrap();
        assert!(s.list_items_for_session(&sid).unwrap().is_empty());
        assert!(matches!(s.delete_item(&item.id), Err(CoreError::NotFound { .. })));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core items`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `src/store/items.rs`)

```rust
use rusqlite::Row;

use crate::domain::CapturedItem;
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

const ITEM_COLS: &str =
    "id, session_id, kind, text, done, created_at, updated_at, device_id";

fn item_from_row(row: &Row) -> Result<CapturedItem, rusqlite::Error> {
    Ok(CapturedItem {
        id: row.get("id")?,
        session_id: row.get("session_id")?,
        kind: row.get("kind")?,
        text: row.get("text")?,
        done: row.get::<_, i64>("done")? != 0,
        created_at: row.get::<_, i64>("created_at")? as u64,
        updated_at: row.get::<_, i64>("updated_at")? as u64,
        device_id: row.get("device_id")?,
    })
}

impl Store {
    /// Adds an item to a session. Works for agent extraction (Plans 04/05)
    /// and manual entry alike (story 10: manual parity — nothing is agent-only).
    pub fn add_item(&self, session_id: &str, kind: &str, text: &str) -> Result<CapturedItem, CoreError> {
        self.get_session(session_id)?; // NotFound if missing/tombstoned
        let now = self.now();
        let item = CapturedItem {
            id: new_id(),
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
            done: false,
            created_at: now,
            updated_at: now,
            device_id: self.device_id.clone(),
        };
        self.conn.execute(
            "INSERT INTO items (id, session_id, kind, text, done, created_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7)",
            rusqlite::params![
                item.id,
                item.session_id,
                item.kind,
                item.text,
                item.created_at as i64,
                item.updated_at as i64,
                item.device_id,
            ],
        )?;
        Ok(item)
    }

    pub fn set_item_done(&self, id: &str, done: bool) -> Result<CapturedItem, CoreError> {
        let changed = self.conn.execute(
            "UPDATE items SET done = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            rusqlite::params![done as i64, self.now() as i64, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "item", id: id.to_string() });
        }
        self.get_item(id)
    }

    pub fn get_item(&self, id: &str) -> Result<CapturedItem, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ITEM_COLS} FROM items WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => item_from_row(row).map_err(CoreError::Sqlite),
            None => Err(CoreError::NotFound { entity: "item", id: id.to_string() }),
        }
    }

    /// Items of one session in insertion order (UUIDv7 ids sort by creation).
    pub fn list_items_for_session(&self, session_id: &str) -> Result<Vec<CapturedItem>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ITEM_COLS} FROM items
             WHERE session_id = ?1 AND deleted_at IS NULL ORDER BY id ASC"
        ))?;
        let mut rows = stmt.query([session_id])?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(item_from_row(row).map_err(CoreError::Sqlite)?);
        }
        Ok(items)
    }

    /// The "morning glance" query (story 1): open todos across all sessions,
    /// oldest first.
    pub fn list_open_todos(&self) -> Result<Vec<CapturedItem>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ITEM_COLS} FROM items
             WHERE kind = 'todo' AND done = 0 AND deleted_at IS NULL ORDER BY id ASC"
        ))?;
        let mut rows = stmt.query([])?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(item_from_row(row).map_err(CoreError::Sqlite)?);
        }
        Ok(items)
    }

    pub fn delete_item(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let changed = self.conn.execute(
            "UPDATE items SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "item", id: id.to_string() });
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): captured items with done tracking and cross-session open-todos query"
```

---

### Task 6: Contacts + Artifacts

**Files:**
- Fill: `src/store/contacts.rs`, `src/store/artifacts.rs`

- [ ] **Step 1: Write the failing tests**

Bottom of `src/store/contacts.rs`:
```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::error::CoreError;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000))
    }

    #[test]
    fn upsert_inserts_then_updates_by_name() {
        let s = store();
        let dev = s.upsert_contact("Dev", Some("framer"), None, None).unwrap();
        assert_eq!(dev.trade.as_deref(), Some("framer"));
        // same name (case-insensitive): update, don't duplicate
        let dev2 = s.upsert_contact("dev", None, Some("555-0100"), None).unwrap();
        assert_eq!(dev2.id, dev.id);
        assert_eq!(dev2.trade.as_deref(), Some("framer"), "None never clears a field");
        assert_eq!(dev2.phone.as_deref(), Some("555-0100"));
        assert_eq!(s.list_contacts().unwrap().len(), 1);
    }

    #[test]
    fn list_orders_by_name() {
        let s = store();
        s.upsert_contact("Zed", None, None, None).unwrap();
        s.upsert_contact("Ana", None, None, None).unwrap();
        let names: Vec<_> = s.list_contacts().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["Ana", "Zed"]);
    }

    #[test]
    fn delete_contact_is_a_tombstone_and_frees_the_name() {
        let s = store();
        let dev = s.upsert_contact("Dev", None, None, None).unwrap();
        s.delete_contact(&dev.id).unwrap();
        assert!(s.list_contacts().unwrap().is_empty());
        assert!(matches!(s.delete_contact(&dev.id), Err(CoreError::NotFound { .. })));
        // upsert after delete creates a NEW contact (tombstone doesn't block the name)
        let dev2 = s.upsert_contact("Dev", None, None, None).unwrap();
        assert_ne!(dev2.id, dev.id);
    }
}
```

Bottom of `src/store/artifacts.rs`:
```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::error::CoreError;
    use crate::store::Store;

    fn store_with_session() -> (Store, String) {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let session = s.start_session(None).unwrap();
        (s, session.id)
    }

    #[test]
    fn add_and_list_for_session() {
        let (s, sid) = store_with_session();
        let report = s
            .add_artifact(&sid, "report", "Johnson walk", "## Summary\nDeck needs work.")
            .unwrap();
        assert_eq!(report.kind, "report");
        let listed = s.list_artifacts_for_session(&sid).unwrap();
        assert_eq!(listed, vec![report]);
    }

    #[test]
    fn add_to_missing_session_is_not_found() {
        let (s, _) = store_with_session();
        assert!(matches!(
            s.add_artifact("nope", "report", "t", "b"),
            Err(CoreError::NotFound { entity: "session", .. })
        ));
    }

    #[test]
    fn update_body_touches_updated_at() {
        let (s, sid) = store_with_session();
        let a = s.add_artifact(&sid, "report", "t", "v1").unwrap();
        let s = s.with_clock(Arc::new(|| 2000));
        let a2 = s.update_artifact_body(&a.id, "v2").unwrap();
        assert_eq!(a2.body, "v2");
        assert_eq!(a2.updated_at, 2000);
    }

    #[test]
    fn delete_artifact_is_a_tombstone() {
        let (s, sid) = store_with_session();
        let a = s.add_artifact(&sid, "report", "t", "b").unwrap();
        s.delete_artifact(&a.id).unwrap();
        assert!(s.list_artifacts_for_session(&sid).unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core contacts` then `... artifacts`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/store/contacts.rs` (above tests):
```rust
use rusqlite::Row;

use crate::domain::Contact;
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

const CONTACT_COLS: &str = "id, name, trade, phone, notes, created_at, updated_at, device_id";

fn contact_from_row(row: &Row) -> Result<Contact, rusqlite::Error> {
    Ok(Contact {
        id: row.get("id")?,
        name: row.get("name")?,
        trade: row.get("trade")?,
        phone: row.get("phone")?,
        notes: row.get("notes")?,
        created_at: row.get::<_, i64>("created_at")? as u64,
        updated_at: row.get::<_, i64>("updated_at")? as u64,
        device_id: row.get("device_id")?,
    })
}

impl Store {
    /// Insert-or-update by case-insensitive name (story 7: contact cards
    /// auto-built from sessions — Plan 04's upsert_contact tool calls this).
    /// `None` fields never clear existing values.
    pub fn upsert_contact(
        &self,
        name: &str,
        trade: Option<&str>,
        phone: Option<&str>,
        notes: Option<&str>,
    ) -> Result<Contact, CoreError> {
        let now = self.now();
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM contacts WHERE lower(name) = lower(?1) AND deleted_at IS NULL",
                [name],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;

        match existing {
            Some(id) => {
                self.conn.execute(
                    "UPDATE contacts SET
                        trade = COALESCE(?1, trade),
                        phone = COALESCE(?2, phone),
                        notes = COALESCE(?3, notes),
                        updated_at = ?4
                     WHERE id = ?5",
                    rusqlite::params![trade, phone, notes, now as i64, id],
                )?;
                self.get_contact(&id)
            }
            None => {
                let contact = Contact {
                    id: new_id(),
                    name: name.to_string(),
                    trade: trade.map(str::to_string),
                    phone: phone.map(str::to_string),
                    notes: notes.map(str::to_string),
                    created_at: now,
                    updated_at: now,
                    device_id: self.device_id.clone(),
                };
                self.conn.execute(
                    "INSERT INTO contacts (id, name, trade, phone, notes, created_at, updated_at, device_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        contact.id,
                        contact.name,
                        contact.trade,
                        contact.phone,
                        contact.notes,
                        contact.created_at as i64,
                        contact.updated_at as i64,
                        contact.device_id,
                    ],
                )?;
                Ok(contact)
            }
        }
    }

    pub fn get_contact(&self, id: &str) -> Result<Contact, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONTACT_COLS} FROM contacts WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => contact_from_row(row).map_err(CoreError::Sqlite),
            None => Err(CoreError::NotFound { entity: "contact", id: id.to_string() }),
        }
    }

    pub fn list_contacts(&self) -> Result<Vec<Contact>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONTACT_COLS} FROM contacts WHERE deleted_at IS NULL ORDER BY name COLLATE NOCASE"
        ))?;
        let mut rows = stmt.query([])?;
        let mut contacts = Vec::new();
        while let Some(row) = rows.next()? {
            contacts.push(contact_from_row(row).map_err(CoreError::Sqlite)?);
        }
        Ok(contacts)
    }

    pub fn delete_contact(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let changed = self.conn.execute(
            "UPDATE contacts SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "contact", id: id.to_string() });
        }
        Ok(())
    }
}
```

`src/store/artifacts.rs` (above tests):
```rust
use rusqlite::Row;

use crate::domain::Artifact;
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

const ARTIFACT_COLS: &str =
    "id, session_id, kind, title, body, created_at, updated_at, device_id";

fn artifact_from_row(row: &Row) -> Result<Artifact, rusqlite::Error> {
    Ok(Artifact {
        id: row.get("id")?,
        session_id: row.get("session_id")?,
        kind: row.get("kind")?,
        title: row.get("title")?,
        body: row.get("body")?,
        created_at: row.get::<_, i64>("created_at")? as u64,
        updated_at: row.get::<_, i64>("updated_at")? as u64,
        device_id: row.get("device_id")?,
    })
}

impl Store {
    /// The artifact seam (Rev 2 §1): any generated document hangs off a
    /// session. Generators register in Plan 04; the store doesn't care what
    /// `kind` means.
    pub fn add_artifact(
        &self,
        session_id: &str,
        kind: &str,
        title: &str,
        body: &str,
    ) -> Result<Artifact, CoreError> {
        self.get_session(session_id)?;
        let now = self.now();
        let artifact = Artifact {
            id: new_id(),
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            created_at: now,
            updated_at: now,
            device_id: self.device_id.clone(),
        };
        self.conn.execute(
            "INSERT INTO artifacts (id, session_id, kind, title, body, created_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                artifact.id,
                artifact.session_id,
                artifact.kind,
                artifact.title,
                artifact.body,
                artifact.created_at as i64,
                artifact.updated_at as i64,
                artifact.device_id,
            ],
        )?;
        Ok(artifact)
    }

    /// Voice edits against artifacts ("make that fourteen hundred") land here
    /// via Plan 04 tools; manual edits use the same path (story 10).
    pub fn update_artifact_body(&self, id: &str, body: &str) -> Result<Artifact, CoreError> {
        let changed = self.conn.execute(
            "UPDATE artifacts SET body = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            rusqlite::params![body, self.now() as i64, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "artifact", id: id.to_string() });
        }
        self.get_artifact(id)
    }

    pub fn get_artifact(&self, id: &str) -> Result<Artifact, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ARTIFACT_COLS} FROM artifacts WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => artifact_from_row(row).map_err(CoreError::Sqlite),
            None => Err(CoreError::NotFound { entity: "artifact", id: id.to_string() }),
        }
    }

    pub fn list_artifacts_for_session(&self, session_id: &str) -> Result<Vec<Artifact>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ARTIFACT_COLS} FROM artifacts
             WHERE session_id = ?1 AND deleted_at IS NULL ORDER BY id ASC"
        ))?;
        let mut rows = stmt.query([session_id])?;
        let mut artifacts = Vec::new();
        while let Some(row) = rows.next()? {
            artifacts.push(artifact_from_row(row).map_err(CoreError::Sqlite)?);
        }
        Ok(artifacts)
    }

    pub fn delete_artifact(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let changed = self.conn.execute(
            "UPDATE artifacts SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "artifact", id: id.to_string() });
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): contact upsert-by-name and artifact seam CRUD"
```

---

### Task 7: Session search

**Files:**
- Modify: `src/store/sessions.rs` (add method + tests)

- [ ] **Step 1: Write the failing tests** (add to the tests module in sessions.rs)

```rust
    #[test]
    fn search_matches_transcript_and_summary() {
        let s = store();
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "the french drain needs regrading").unwrap();
        s.end_session(&a.id).unwrap();

        let b = s.start_session(None).unwrap();
        s.end_session(&b.id).unwrap();
        s.mark_session_processed(&b.id, "Discussed drain pricing with Johnson").unwrap();

        let c = s.start_session(None).unwrap();
        s.append_transcript(&c.id, "unrelated roofing talk").unwrap();

        let hits: Vec<_> = s.search_sessions("drain").unwrap().into_iter().map(|x| x.id).collect();
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&a.id) && hits.contains(&b.id));
    }

    #[test]
    fn search_is_case_insensitive_for_ascii() {
        let s = store();
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "French Drain").unwrap();
        assert_eq!(s.search_sessions("french").unwrap().len(), 1);
    }

    #[test]
    fn search_escapes_like_metacharacters() {
        let s = store();
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "50% deposit due").unwrap();
        let b = s.start_session(None).unwrap();
        s.append_transcript(&b.id, "500 deposit due").unwrap();
        let hits = s.search_sessions("50%").unwrap();
        assert_eq!(hits.len(), 1, "% must match literally, not as wildcard");
        assert_eq!(hits[0].id, a.id);
    }

    #[test]
    fn empty_or_blank_query_returns_nothing() {
        let s = store();
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "anything").unwrap();
        assert!(s.search_sessions("").unwrap().is_empty());
        assert!(s.search_sessions("   ").unwrap().is_empty());
    }
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core search`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (add to sessions.rs)

```rust
/// Escapes LIKE metacharacters so user text matches literally.
fn like_pattern(query: &str) -> String {
    let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}
```

And in `impl Store`:
```rust
    /// Session-library text search (story 9) over transcripts and summaries,
    /// newest first. Plain LIKE — case-insensitive for ASCII only; an FTS5
    /// upgrade is the seam if real usage (~100+ sessions) demands it
    /// (research rec #7: wait for evidence).
    pub fn search_sessions(&self, query: &str) -> Result<Vec<Session>, CoreError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let pattern = like_pattern(query);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions
             WHERE deleted_at IS NULL
               AND (transcript LIKE ?1 ESCAPE '\\' OR summary LIKE ?1 ESCAPE '\\')
             ORDER BY started_at DESC, id DESC"
        ))?;
        let mut rows = stmt.query([&pattern])?;
        let mut sessions = Vec::new();
        while let Some(row) = rows.next()? {
            sessions.push(session_from_row(row)?);
        }
        Ok(sessions)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): session library text search with LIKE escaping"
```

---

### Task 8: Reflection state persistence + activity derivation

**Files:**
- Create: `src/reflection.rs`
- Modify: `src/lib.rs` (`pub mod reflection;` — no new re-exports; methods live on Store)

- [ ] **Step 1: Write the failing tests** (bottom of `src/reflection.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000))
    }

    #[test]
    fn signals_default_when_never_reflected() {
        let s = store();
        let signals = s.reflection_signals().unwrap();
        assert_eq!(signals.sessions_since_reflection, 0);
        assert_eq!(signals.completed_reflections, 0);
    }

    #[test]
    fn session_and_correction_counters_persist() {
        let s = store();
        s.record_session_completed().unwrap();
        s.record_session_completed().unwrap();
        s.record_correction().unwrap();
        let signals = s.reflection_signals().unwrap();
        assert_eq!(signals.sessions_since_reflection, 2);
        assert_eq!(signals.corrections_since_reflection, 1);
    }

    #[test]
    fn record_reflection_resets_and_stamps_time() {
        let s = store();
        s.record_session_completed().unwrap();
        s.record_correction().unwrap();
        s.record_reflection(0.4).unwrap();
        let signals = s.reflection_signals().unwrap();
        assert_eq!(signals.sessions_since_reflection, 0);
        assert_eq!(signals.corrections_since_reflection, 0);
        assert_eq!(signals.completed_reflections, 1);
        assert_eq!(signals.recent_churn, vec![0.4]);
        assert_eq!(s.last_reflected_at().unwrap(), 1000);
    }

    #[test]
    fn activity_uses_summary_else_transcript_excerpt() {
        let s = store();
        // processed session with summary
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "long raw words").unwrap();
        s.end_session(&a.id).unwrap();
        s.mark_session_processed(&a.id, "Walked the deck; 2 todos.").unwrap();
        // ended but unprocessed session — falls back to transcript excerpt
        let b = s.start_session(None).unwrap();
        s.append_transcript(&b.id, "call Dev about framing").unwrap();
        s.end_session(&b.id).unwrap();
        // still recording — excluded
        let c = s.start_session(None).unwrap();
        s.append_transcript(&c.id, "in progress").unwrap();

        let activity = s.activity_for_reflection(10).unwrap();
        assert_eq!(activity.len(), 2);
        assert!(activity[0].contains("Walked the deck"));
        assert!(activity[1].contains("call Dev about framing"));
        let _ = c;
    }

    #[test]
    fn activity_excludes_sessions_before_last_reflection() {
        let s = store();
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "old session").unwrap();
        s.end_session(&a.id).unwrap();

        let s = s.with_clock(Arc::new(|| 2000));
        s.record_reflection(0.1).unwrap();

        let s = s.with_clock(Arc::new(|| 3000));
        let b = s.start_session(None).unwrap();
        s.append_transcript(&b.id, "new session words").unwrap();
        s.end_session(&b.id).unwrap();

        let activity = s.activity_for_reflection(10).unwrap();
        assert_eq!(activity.len(), 1);
        assert!(activity[0].contains("new session words"));
    }

    #[test]
    fn activity_respects_limit_and_excerpts_are_bounded() {
        let s = store();
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, &"word ".repeat(400)).unwrap();
        s.end_session(&a.id).unwrap();
        let b = s.start_session(None).unwrap();
        s.append_transcript(&b.id, "short").unwrap();
        s.end_session(&b.id).unwrap();

        let activity = s.activity_for_reflection(1).unwrap();
        assert_eq!(activity.len(), 1, "limit keeps the MOST RECENT sessions");
        assert!(activity[0].contains("short"));

        let all = s.activity_for_reflection(10).unwrap();
        let long_entry = all.iter().find(|e| e.contains("word")).unwrap();
        assert!(long_entry.chars().count() <= 560, "excerpt is bounded (~500 chars + prefix)");
    }

    #[test]
    fn job_name_prefixes_activity_when_linked() {
        let s = store();
        let job = s
            .create_job(crate::domain::NewJob { name: "Johnson remodel".into(), ..Default::default() })
            .unwrap();
        let a = s.start_session(Some(&job.id)).unwrap();
        s.append_transcript(&a.id, "measured the kitchen").unwrap();
        s.end_session(&a.id).unwrap();
        let activity = s.activity_for_reflection(10).unwrap();
        assert!(activity[0].starts_with("[Johnson remodel] "));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core reflection`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `src/reflection.rs`)

```rust
//! Persistence and derivation of reflection signals (spec §7, Rev 3 §1).
//! The harness owns the cadence policy and `ReflectionSignals` itself;
//! murmur-core persists the counters and derives the activity feed from the
//! session library. The reflection COORDINATOR (snapshot memory → run
//! ReflectionEngine → swap-and-persist → record) is Plan 04 — reflection must
//! not overlap an active session (Plan 02 engine doc).

use harness::ReflectionSignals;

use crate::error::CoreError;
use crate::store::Store;

/// Cap on each activity entry fed to reflection (chars, before the job prefix).
const EXCERPT_CHARS: usize = 500;

impl Store {
    pub fn reflection_signals(&self) -> Result<ReflectionSignals, CoreError> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT signals FROM reflection_state WHERE id = 1", [], |r| r.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        match raw {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(ReflectionSignals::default()),
        }
    }

    pub fn last_reflected_at(&self) -> Result<u64, CoreError> {
        let raw: Option<i64> = self
            .conn
            .query_row(
                "SELECT last_reflected_at FROM reflection_state WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(raw.unwrap_or(0) as u64)
    }

    fn save_signals(&self, signals: &ReflectionSignals, reflected_at: Option<u64>) -> Result<(), CoreError> {
        let json = serde_json::to_string(signals)?;
        match reflected_at {
            Some(at) => self.conn.execute(
                "INSERT INTO reflection_state (id, signals, last_reflected_at) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET signals = ?1, last_reflected_at = ?2",
                rusqlite::params![json, at as i64],
            )?,
            None => self.conn.execute(
                "INSERT INTO reflection_state (id, signals, last_reflected_at) VALUES (1, ?1, 0)
                 ON CONFLICT(id) DO UPDATE SET signals = ?1",
                rusqlite::params![json],
            )?,
        };
        Ok(())
    }

    /// Call when a session ends (the app layer's session-end hook).
    pub fn record_session_completed(&self) -> Result<ReflectionSignals, CoreError> {
        let mut signals = self.reflection_signals()?;
        signals.sessions_since_reflection += 1;
        self.save_signals(&signals, None)?;
        Ok(signals)
    }

    /// Call when the user corrects agent output (R7 / Rev 3 §1: corrections
    /// snap reflection cadence back).
    pub fn record_correction(&self) -> Result<ReflectionSignals, CoreError> {
        let mut signals = self.reflection_signals()?;
        signals.corrections_since_reflection += 1;
        self.save_signals(&signals, None)?;
        Ok(signals)
    }

    /// Call after a completed reflection. Delegates counter semantics to
    /// `harness::ReflectionSignals::record_reflection` and stamps the time.
    pub fn record_reflection(&self, churn: f32) -> Result<(), CoreError> {
        let mut signals = self.reflection_signals()?;
        signals.record_reflection(churn);
        self.save_signals(&signals, Some(self.now()))?;
        Ok(())
    }

    /// Activity feed for `ReflectionEngine::reflect`: ended sessions since the
    /// last reflection, oldest→newest, at most `max_sessions` MOST RECENT.
    /// Uses the pipeline summary when present, else a bounded transcript
    /// excerpt; sessions still recording are excluded. Linked job names are
    /// prefixed so reflection can learn project vocabulary.
    pub fn activity_for_reflection(&self, max_sessions: usize) -> Result<Vec<String>, CoreError> {
        let since = self.last_reflected_at()? as i64;
        let mut stmt = self.conn.prepare(
            "SELECT s.summary, s.transcript, j.name
             FROM sessions s LEFT JOIN jobs j ON j.id = s.job_id
             WHERE s.deleted_at IS NULL AND s.ended_at IS NOT NULL AND s.ended_at > ?1
             ORDER BY s.ended_at DESC, s.id DESC
             LIMIT ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![since, max_sessions as i64])?;
        let mut entries: Vec<String> = Vec::new();
        while let Some(row) = rows.next()? {
            let summary: Option<String> = row.get(0)?;
            let transcript: String = row.get(1)?;
            let job_name: Option<String> = row.get(2)?;
            let body = match summary {
                Some(s) if !s.trim().is_empty() => s,
                _ => transcript.chars().take(EXCERPT_CHARS).collect(),
            };
            if body.trim().is_empty() {
                continue;
            }
            entries.push(match job_name {
                Some(name) => format!("[{name}] {body}"),
                None => body,
            });
        }
        entries.reverse(); // oldest → newest, matching ReflectionEngine's numbering
        Ok(entries)
    }
}
```

`src/lib.rs` — add `pub mod reflection;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core` and `cargo test` (whole workspace).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): reflection signal persistence and session-derived activity feed"
```

---

### Task 9: Docs + full verification

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README**

Update the plan-series lines to:
```markdown
Done: 01 foundation, 02 memory + reflection + context assembler, 03 domain + storage.
Next: 04 processing pipeline (vocational tools, reflection coordinator).
```
Add a one-line crate description under the existing crate list (match README style): `murmur-core` — domain entities + sync-ready SQLite storage (single-writer API, tombstones, UUIDv7).

- [ ] **Step 2: Full verification**

Run: `cargo test` → all pass (harness's 64 + murmur-core's new suite). Run: `cargo clippy --all-targets` → zero warnings. Fix clippy findings mechanically (no public API changes); if a fix would change behavior, STOP and report.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "docs: plan series status; clippy clean for plan 03"
```

---

## Deferred (named, for Plan 04)

Carried from Plan 02's final review + this plan's scope cuts:

1. **Reflection coordinator** — snapshot memory (FileMemoryStore already rotates, but take an explicit pre-reflection copy), gather `activity_for_reflection`, run `ReflectionEngine::reflect`, swap-and-persist, `Store::record_reflection(churn)`. Must not overlap an active session.
2. **Processing pipeline** — end-of-session agent pass with vocational tools (`create_report`, `update_todos`, `upsert_contact`, `update_memory`) writing through this plan's Store methods; `mark_session_processed`/`mark_session_failed` are its exits. R6 (under-extraction bias) and R7 (inspectable/undoable) live in its prompts/tools.
3. **NFC normalization** — if keyboard input joins STT input, normalize text before memory writes and search.
4. **FTS5 search upgrade** — seam documented on `search_sessions`; wait for ~100+ sessions evidence.
5. **Audio retention policy** (spec §8: discard audio after 7 days) — belongs with the audio layer (STT plan), not storage of transcripts.

## Self-Review Notes

- **Spec coverage:** §9 UUIDv7 ✓ (Task 1), timestamps+device_id on every row ✓ (schema, Task 2), tombstones ✓ (all deletes), single writer API ✓ (Store methods only, doc'd), migrations ✓ (user_version runner); Rev 2 Job first-class ✓ (Task 3), artifact seam ✓ (Task 6); stories: 7 contacts ✓ (Task 6), 9 library+search ✓ (Tasks 4/7), 10 manual parity ✓ (same Store methods serve agent and hand entry); §7 reflection signals persistence + activity ✓ (Task 8). Out of scope by design: LLM pipeline, layout, FFI, sync engine.
- **Type consistency:** `CoreError` variants defined Task 1, used throughout; `Store` fields (`conn`, `device_id`, `clock`) defined Task 2, used by all impl blocks; `SESSION_COLS`/`session_from_row` defined Task 4, reused by Task 7's search; `harness::Clock` and `harness::ReflectionSignals` are existing exports (verified against Plan 02's shipped lib.rs); `NewJob` derives Default so struct-update syntax in tests compiles.
- **Judgment calls for reviewers:** status enums parse strictly (`Corrupt` on unknown) — the DB is device-local and migration-versioned, lenient parsing is for network edges, not our own disk; `kind` on items/artifacts is a free string — new kinds must not need migrations; LIKE search over FTS5 — evidence-first (research rec #7); `done` on all item kinds (harmless for notes, uniform API); contact upsert matches case-insensitive exact name — fuzzy matching is a Plan 04+ product question.
- **Test-count checkpoints:** T1 +2, T2 +4, T3 +6, T4 +8, T5 +5, T6 +7, T7 +4, T8 +7 ≈ 43 new murmur-core tests. Counts are expectations, not gates.
