use rusqlite::Connection;

use crate::error::CoreError;

/// One entry per schema version, applied in order. NEVER edit an existing
/// entry after it has shipped — append a new one.
pub(crate) const MIGRATIONS: &[&str] = &[
    // v1: initial schema (spec §9: timestamps + device id on every row, tombstones)
    r#"
    -- all *_at columns are unix epoch-seconds
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

    -- local-only bookkeeping: not synced, so no timestamps/device_id/tombstone
    CREATE TABLE reflection_state (
        id                INTEGER PRIMARY KEY CHECK (id = 1),
        signals           TEXT NOT NULL,
        last_reflected_at INTEGER NOT NULL DEFAULT 0
    );

    -- append-only cost log (R9: cost per session measured from day one).
    -- No tombstone: rows are never deleted, only summed.
    CREATE TABLE llm_usage (
        id            TEXT PRIMARY KEY,
        session_id    TEXT REFERENCES sessions(id),
        purpose       TEXT NOT NULL,
        input_tokens  INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        created_at    INTEGER NOT NULL,
        device_id     TEXT NOT NULL
    );
    CREATE INDEX idx_llm_usage_session ON llm_usage(session_id);

    CREATE INDEX idx_jobs_scheduled ON jobs(scheduled_at);
    CREATE INDEX idx_sessions_started ON sessions(started_at) WHERE deleted_at IS NULL;
    CREATE INDEX idx_sessions_job ON sessions(job_id) WHERE deleted_at IS NULL;
    CREATE INDEX idx_items_session ON items(session_id) WHERE deleted_at IS NULL;
    CREATE INDEX idx_artifacts_session ON artifacts(session_id) WHERE deleted_at IS NULL;
    "#,
    // v2: items.source (Plan 06a). Backfill existing rows as 'authoritative' —
    // pre-06a items were all written by the processing pipeline (Plan 04) or by
    // manual add_item; treating them as authoritative is the safe default (they
    // are never swept unless a *new* run supersedes them, exactly today's
    // behavior). SQLite ADD COLUMN with NOT NULL requires the DEFAULT.
    r#"
    ALTER TABLE items ADD COLUMN source TEXT NOT NULL DEFAULT 'authoritative';
    "#,
    // v3: sessions.template (Plan 07 D4) — nullable key selecting extraction
    // vocabulary + document layout ("landscape" | "property" | "inspection").
    // Persisted (not pass-through) so reprocessing stays template-consistent.
    r#"
    ALTER TABLE sessions ADD COLUMN template TEXT;
    "#,
    // v4: document_sequences (Plan 07 D5) — per-doc-kind monotonic counters
    // for minting document numbers (EST-0001, etc.). Local bookkeeping, same
    // posture as reflection_state: no tombstone/sync fields, device-local.
    r#"
    CREATE TABLE document_sequences (
        doc_kind  TEXT PRIMARY KEY,
        next      INTEGER NOT NULL,
        device_id TEXT NOT NULL
    );
    "#,
    // v5: photo attachments (Plan 11). Metadata + a relative filename only; the
    // BYTES live in the shell's Documents dir and never sync (privacy: photos
    // never leave the device, spec §1/§8). Row shape is sync-ready (§9).
    r#"
    CREATE TABLE photos (
        id          TEXT PRIMARY KEY,
        session_id  TEXT NOT NULL REFERENCES sessions(id),
        item_id     TEXT REFERENCES items(id),   -- NULL = session-level attachment
        filename    TEXT NOT NULL,               -- shell-owned, opaque to core (relative name)
        captured_at INTEGER NOT NULL,            -- unix seconds (EXIF shot-time or now())
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        device_id   TEXT NOT NULL,
        deleted_at  INTEGER
    );
    -- At most one LIVE row per filename → the reconciling sweep (D4) is unambiguous.
    CREATE UNIQUE INDEX idx_photos_filename_live ON photos(filename) WHERE deleted_at IS NULL;
    CREATE INDEX idx_photos_session ON photos(session_id) WHERE deleted_at IS NULL;
    "#,
    // v6: items.right_text (Plan 16) — the quantity/unit string ("3 CU YD",
    // "× 4"). Named right_text, NOT right: RIGHT is a SQL keyword (SQLite
    // ≥3.39 RIGHT JOIN), a reserved-word footgun in a bare SELECT column
    // list; the domain field stays `right` (matches BoardItem.right).
    // Quantity, never price — pricing stays document-only (keeper D-#2).
    // ADD COLUMN with NOT NULL requires the DEFAULT (the v2 precedent).
    r#"
    ALTER TABLE items ADD COLUMN right_text TEXT NOT NULL DEFAULT '';
    "#,
    // v7: document_schemas (Plan 19) — document structure becomes data. The
    // TABLE only: the built-ins are seeded by `schemas::seed_builtin_schemas`
    // from `from_connection` AFTER migrate (this framework is pure SQL strings
    // with no Rust hook, and it runs before device_id/clock exist), iterating
    // the ONE source `domain::builtin_schemas()`. Row shape is sync-ready
    // (§9); the structural part is a JSON envelope column (`sections`), the
    // artifacts.body precedent — a future multi-section relaxation is a JSON
    // change, not a migration.
    r#"
    CREATE TABLE document_schemas (
        id             TEXT PRIMARY KEY,
        kind           TEXT NOT NULL,
        label          TEXT NOT NULL,
        number_prefix  TEXT NOT NULL,
        trade_key      TEXT,
        sections       TEXT NOT NULL,
        schema_version INTEGER NOT NULL,
        created_at     INTEGER NOT NULL,
        updated_at     INTEGER NOT NULL,
        device_id      TEXT NOT NULL,
        deleted_at     INTEGER
    );
    CREATE INDEX idx_document_schemas_kind ON document_schemas(kind) WHERE deleted_at IS NULL;
    "#,
    // v8: prompt-cache token columns on llm_usage (R9 stays honest under
    // caching). The API reports cache tokens in SEPARATE fields and collapses
    // `input_tokens` to the uncached remainder — so without these two columns,
    // switching caching on would make the spend meter read like a huge saving
    // that never happened. Additive with DEFAULT 0: every pre-v8 row is
    // correct as-is, because those calls genuinely cached nothing.
    r#"
    ALTER TABLE llm_usage ADD COLUMN cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE llm_usage ADD COLUMN cache_read_input_tokens INTEGER NOT NULL DEFAULT 0;
    "#,
];

pub(crate) fn migrate(conn: &Connection) -> Result<(), CoreError> {
    migrate_with(conn, MIGRATIONS)
}

/// Applies pending migrations from `migrations`. Each one is all-or-nothing:
/// the DDL and the `user_version` bump commit in a single transaction, so a
/// mid-batch failure rolls back cleanly instead of leaving partial tables
/// behind with a stale version.
fn migrate_with(conn: &Connection, migrations: &[&str]) -> Result<(), CoreError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in migrations.iter().enumerate().skip(version as usize) {
        let result = conn.execute_batch(&format!(
            "BEGIN;\n{}\nPRAGMA user_version = {};\nCOMMIT;",
            sql,
            i + 1
        ));
        if let Err(e) = result {
            // A mid-batch failure leaves the explicit BEGIN open on the
            // connection; roll it back so the connection stays usable.
            if !conn.is_autocommit() {
                conn.execute_batch("ROLLBACK;")?;
            }
            return Err(e.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v8_upgrade_preserves_existing_llm_usage_rows() {
        // Every device already in the field is at v7. v8 must add the two cache
        // columns without disturbing rows written before it — those calls
        // genuinely cached nothing, so 0 is the honest value, not a placeholder.
        let conn = Connection::open_in_memory().unwrap();

        // Bring the DB to v7 only, then write a row the pre-v8 way.
        let v7: Vec<&str> = MIGRATIONS.iter().take(7).copied().collect();
        migrate_with(&conn, &v7).unwrap();
        conn.execute(
            "INSERT INTO llm_usage (id, session_id, purpose, input_tokens, output_tokens,
                                    created_at, device_id)
             VALUES ('u1', NULL, 'processing', 900, 120, 1000, 'device-a')",
            [],
        )
        .unwrap();

        // Apply the full set — v8 lands on top of that existing row.
        migrate_with(&conn, MIGRATIONS).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 8);

        let (input, cw, cr, output): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT input_tokens, cache_creation_input_tokens,
                        cache_read_input_tokens, output_tokens
                 FROM llm_usage WHERE id = 'u1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((input, output), (900, 120), "existing counts untouched");
        assert_eq!((cw, cr), (0, 0), "new columns default to 0, not NULL");
    }

    #[test]
    fn failed_migration_rolls_back_cleanly() {
        let conn = Connection::open_in_memory().unwrap();
        let broken: &[&str] = &[MIGRATIONS[0], "CREATE TABLE broken (;"];
        let err = migrate_with(&conn, broken);
        assert!(err.is_err(), "broken migration must surface an error");

        // v1 committed; the broken v2 rolled back entirely.
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);

        // Re-running with a fixed second migration succeeds.
        let fixed: &[&str] = &[MIGRATIONS[0], "CREATE TABLE fixed (id TEXT PRIMARY KEY);"];
        migrate_with(&conn, fixed).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }
}
