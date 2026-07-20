//! `document_schemas` storage (Plan 19): document structure as data. Seeded
//! built-ins + CRUD + resolution, mirroring `store/items.rs`'s sync-ready row
//! discipline (created_at/updated_at/device_id, tombstones, guarded reads).
//!
//! Seeding runs on EVERY `Store::open` (from `from_connection`, after
//! migrate) — the resurrection guard is LIVE on every launch: a new built-in
//! added to `domain::builtin_schemas()` seeds naturally on the next open, and
//! a deleted (tombstoned) built-in stays deleted forever (WE-A). The v7
//! migration creates the TABLE only.

use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};

use crate::domain::{builtin_schemas, DocumentSchema, SchemaSection};
use crate::error::CoreError;
use crate::store::Store;

const SCHEMA_COLS: &str = "id, kind, label, number_prefix, trade_key, sections, schema_version, \
                           created_at, updated_at, device_id";

/// The persisted JSON shape of the `sections` column (Plan 19 §3): the
/// envelope carries the total shape alongside the ordered sections so the
/// flexible structural part stays ONE column (the `artifacts.body` precedent).
#[derive(Serialize, Deserialize)]
struct SectionsEnvelope {
    total_kind: String,
    total_label_key: String,
    sections: Vec<SchemaSection>,
}

fn envelope_json(schema: &DocumentSchema) -> Result<String, CoreError> {
    Ok(serde_json::to_string(&SectionsEnvelope {
        total_kind: schema.total_kind.clone(),
        total_label_key: schema.total_label_key.clone(),
        sections: schema.sections.clone(),
    })?)
}

fn schema_from_row(row: &Row) -> Result<DocumentSchema, CoreError> {
    let envelope_raw: String = row.get("sections").map_err(CoreError::Sqlite)?;
    let envelope: SectionsEnvelope = serde_json::from_str(&envelope_raw)
        .map_err(|e| CoreError::Corrupt(format!("bad document_schemas.sections JSON: {e}")))?;
    Ok(DocumentSchema {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        kind: row.get("kind").map_err(CoreError::Sqlite)?,
        label: row.get("label").map_err(CoreError::Sqlite)?,
        number_prefix: row.get("number_prefix").map_err(CoreError::Sqlite)?,
        trade_key: row.get("trade_key").map_err(CoreError::Sqlite)?,
        total_kind: envelope.total_kind,
        total_label_key: envelope.total_label_key,
        sections: envelope.sections,
        schema_version: row.get::<_, i64>("schema_version").map_err(CoreError::Sqlite)? as u32,
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        updated_at: row.get::<_, i64>("updated_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
    })
}

/// Seeds the built-in schemas (Plan 19 Stage 1). Runs on EVERY store open,
/// iterating the ONE source `builtin_schemas()`; each row is a parameterized,
/// tombstone-respecting insert — the `WHERE NOT EXISTS` checks EVERY row
/// including tombstoned ones, so a soft-deleted built-in blocks its own
/// re-seed forever (the resurrection guard, WE-A). Seeded rows carry the
/// sentinel `device_id` ("builtin") and fixed timestamps (0) so they are
/// byte-identical on every device (the stable sync merge key).
pub(crate) fn seed_builtin_schemas(conn: &Connection) -> Result<(), CoreError> {
    seed_schemas(conn, &builtin_schemas())
}

/// The seeding mechanism itself, factored so tests can drive it with an
/// EXTENDED built-in list (the WE-A "app-update adds punch_list" trace)
/// through the exact code path production uses.
pub(crate) fn seed_schemas(conn: &Connection, schemas: &[DocumentSchema]) -> Result<(), CoreError> {
    for schema in schemas {
        let sections = envelope_json(schema)?;
        conn.execute(
            "INSERT INTO document_schemas
                 (id, kind, label, number_prefix, trade_key, sections, schema_version,
                  created_at, updated_at, device_id, deleted_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL
             WHERE NOT EXISTS (SELECT 1 FROM document_schemas WHERE id = ?1)",
            rusqlite::params![
                schema.id,
                schema.kind,
                schema.label,
                schema.number_prefix,
                schema.trade_key,
                sections,
                schema.schema_version as i64,
                schema.created_at as i64,
                schema.updated_at as i64,
                schema.device_id,
            ],
        )?;
    }
    Ok(())
}

/// Save-time validation (Plan 19 Stage 3, R6 reject-never-coerce): runs at
/// the top of `save_document_schema`, BEFORE any write — nothing persists on
/// rejection. Rejects unknown section/field/fill kinds against the ONE shared
/// allowlists (`domain::VALID_*`), ≠1 `line_items` section (v1: reject 0 and
/// 2+ — item-less docs and labor-vs-materials are the named future
/// relaxations), and empty `kind`/`label`/`number_prefix`. Error text mirrors
/// `ffi/items.rs`'s allowlist message shape.
pub(crate) fn validate_schema(schema: &DocumentSchema) -> Result<(), CoreError> {
    use crate::domain::{VALID_FIELD_KINDS, VALID_FILL_KINDS, VALID_SECTION_KINDS};
    if schema.kind.trim().is_empty() {
        return Err(CoreError::InvalidState("schema kind is empty".into()));
    }
    if schema.label.trim().is_empty() {
        return Err(CoreError::InvalidState("schema label is empty".into()));
    }
    if schema.number_prefix.trim().is_empty() {
        return Err(CoreError::InvalidState("schema number_prefix is empty".into()));
    }
    for section in &schema.sections {
        if !VALID_SECTION_KINDS.contains(&section.kind.as_str()) {
            return Err(CoreError::InvalidState(format!(
                "invalid section kind '{}'; must be one of: {}",
                section.kind,
                VALID_SECTION_KINDS.join(", ")
            )));
        }
        for field in &section.fields {
            if !VALID_FIELD_KINDS.contains(&field.kind.as_str()) {
                return Err(CoreError::InvalidState(format!(
                    "invalid field kind '{}'; must be one of: {}",
                    field.kind,
                    VALID_FIELD_KINDS.join(", ")
                )));
            }
            if !VALID_FILL_KINDS.contains(&field.fill.as_str()) {
                return Err(CoreError::InvalidState(format!(
                    "invalid fill '{}'; must be one of: {}",
                    field.fill,
                    VALID_FILL_KINDS.join(", ")
                )));
            }
        }
    }
    let line_items = schema.sections.iter().filter(|s| s.kind == "line_items").count();
    if line_items != 1 {
        return Err(CoreError::InvalidState(format!(
            "a schema must have exactly one line_items section (found {line_items})"
        )));
    }
    Ok(())
}

impl Store {
    /// One live schema by id; `NotFound` for missing or tombstoned rows
    /// (the `get_item` discipline).
    pub fn get_document_schema(&self, id: &str) -> Result<DocumentSchema, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SCHEMA_COLS} FROM document_schemas WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => schema_from_row(row),
            None => Err(CoreError::NotFound { entity: "document_schema", id: id.to_string() }),
        }
    }

    /// Live schemas, ordered by id (fixed built-in ids sort FIRST, then
    /// UUIDv7 customs in creation order). `Some(t)` filters to
    /// `trade_key = t OR trade_key IS NULL` (template-agnostic schemas like
    /// `report` show for every trade); `None` returns all live schemas.
    pub fn list_document_schemas(
        &self,
        trade_key: Option<&str>,
    ) -> Result<Vec<DocumentSchema>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SCHEMA_COLS} FROM document_schemas
             WHERE (?1 IS NULL OR trade_key = ?1 OR trade_key IS NULL)
               AND deleted_at IS NULL
             ORDER BY id ASC"
        ))?;
        let mut rows = stmt.query(rusqlite::params![trade_key])?;
        let mut schemas = Vec::new();
        while let Some(row) = rows.next()? {
            schemas.push(schema_from_row(row)?);
        }
        Ok(schemas)
    }

    /// Upsert by id (Plan 19 Stage 2): an existing LIVE row is updated in
    /// place (editable fields + `updated_at` bump, `created_at`/`device_id`
    /// preserved); otherwise a fresh row is inserted with this store's
    /// device_id and clock. An empty `id` mints a fresh UUIDv7 (the FFI
    /// convenience for "create"). A tombstoned id is NOT silently
    /// resurrected — the update targets live rows only, so saving over a
    /// tombstone surfaces the PK conflict as an error (truthful, R7).
    /// Validation (Stage 3) runs BEFORE any write.
    pub fn save_document_schema(
        &self,
        schema: &DocumentSchema,
    ) -> Result<DocumentSchema, CoreError> {
        validate_schema(schema)?; // R6: reject BEFORE any write (Stage 3)
        let id = if schema.id.trim().is_empty() {
            crate::ids::new_id()
        } else {
            schema.id.clone()
        };
        let sections = envelope_json(schema)?;
        let now = self.now();
        let changed = self.conn.execute(
            "UPDATE document_schemas
             SET kind = ?1, label = ?2, number_prefix = ?3, trade_key = ?4,
                 sections = ?5, schema_version = ?6, updated_at = ?7
             WHERE id = ?8 AND deleted_at IS NULL",
            rusqlite::params![
                schema.kind,
                schema.label,
                schema.number_prefix,
                schema.trade_key,
                sections,
                schema.schema_version as i64,
                now as i64,
                id,
            ],
        )?;
        if changed == 0 {
            self.conn.execute(
                "INSERT INTO document_schemas
                     (id, kind, label, number_prefix, trade_key, sections, schema_version,
                      created_at, updated_at, device_id, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, NULL)",
                rusqlite::params![
                    id,
                    schema.kind,
                    schema.label,
                    schema.number_prefix,
                    schema.trade_key,
                    sections,
                    schema.schema_version as i64,
                    now as i64,
                    self.device_id,
                ],
            )?;
        }
        self.get_document_schema(&id)
    }

    /// Tombstone (the `delete_item` discipline): `NotFound` on a second
    /// remove. A removed BUILT-IN stays removed forever — the seed's
    /// `WHERE NOT EXISTS` sees this tombstone on every subsequent open.
    pub fn remove_document_schema(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let changed = self.conn.execute(
            "UPDATE document_schemas SET deleted_at = ?1, updated_at = ?1
             WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "document_schema", id: id.to_string() });
        }
        Ok(())
    }

    /// The active schema for `(kind, template)` (Plan 19 §4): trade must
    /// match exactly — a NULL-trade schema (e.g. `report`) resolves ONLY for
    /// a None-template session, so it stays illegal on a landscape session,
    /// matching today. Newest `updated_at` wins (a custom save shadows the
    /// fixed-timestamp built-in). `None` when nothing resolves (e.g. an
    /// operator tombstoned a built-in) — the caller fails truthfully, never
    /// falls back to a hardcoded shape (that would resurrect a deleted
    /// built-in).
    pub fn resolve_active_schema(
        &self,
        kind: &str,
        template: Option<&str>,
    ) -> Result<Option<DocumentSchema>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SCHEMA_COLS} FROM document_schemas
             WHERE kind = ?1
               AND (trade_key = ?2 OR (trade_key IS NULL AND ?2 IS NULL))
               AND deleted_at IS NULL
             ORDER BY updated_at DESC LIMIT 1"
        ))?;
        let mut rows = stmt.query(rusqlite::params![kind, template])?;
        match rows.next()? {
            Some(row) => Ok(Some(schema_from_row(row)?)),
            None => Ok(None),
        }
    }

    /// The legality gate's second clause (Plan 19 §4): whether ANY live
    /// schema resolves for `(kind, template)`.
    pub fn has_active_schema(&self, kind: &str, template: Option<&str>) -> Result<bool, CoreError> {
        Ok(self.resolve_active_schema(kind, template)?.is_some())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::{
        SchemaField, BUILTIN_SCHEMA_ID_ESTIMATE, BUILTIN_SCHEMA_ID_INVOICE,
        BUILTIN_SCHEMA_ID_REPORT, BUILTIN_SCHEMA_ID_WORK_ORDER,
    };
    use crate::error::CoreError;
    use crate::pipeline::{is_pricing_kind, total_shape};
    use crate::store::Store;

    /// A minimal valid custom schema (one line_items section) for CRUD tests.
    pub(super) fn custom_schema(id: &str, kind: &str, trade: Option<&str>) -> DocumentSchema {
        DocumentSchema {
            id: id.to_string(),
            kind: kind.to_string(),
            label: kind.to_string(),
            number_prefix: "HOA".to_string(),
            trade_key: trade.map(str::to_string),
            total_kind: "sum".to_string(),
            total_label_key: "total".to_string(),
            sections: vec![SchemaSection {
                key: "line_items".into(),
                kind: "line_items".into(),
                label: "Items".into(),
                priced: false,
                fields: vec![],
            }],
            schema_version: 1,
            created_at: 0,
            updated_at: 0,
            device_id: String::new(),
        }
    }

    #[test]
    fn fresh_store_is_at_schema_v7() {
        let s = Store::open_in_memory("device-a").unwrap();
        let v: i64 =
            s.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 7, "v7 added document_schemas (Plan 19)");
    }

    #[test]
    fn v7_seeds_exactly_the_seven_builtins() {
        let s = Store::open_in_memory("device-a").unwrap();
        let rows: Vec<(String, String, Option<String>, String)> = {
            let mut stmt = s
                .conn
                .prepare(
                    "SELECT id, kind, trade_key, number_prefix FROM document_schemas ORDER BY id",
                )
                .unwrap();
            let got = stmt
                .query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            got
        };
        let expected: Vec<(String, String, Option<String>, String)> = builtin_schemas()
            .into_iter()
            .map(|b| (b.id, b.kind, b.trade_key, b.number_prefix))
            .collect();
        assert_eq!(rows, expected, "exactly the seven built-ins, ids/kinds/trades/prefixes");
        assert_eq!(rows.len(), 7);
    }

    /// The guard that the parameterized INSERT and the `Vec` source never
    /// drift (an inline-SQL duplicate would have risked exactly that).
    #[test]
    fn seeded_rows_deep_equal_builtin_schemas() {
        let s = Store::open_in_memory("device-a").unwrap();
        for builtin in builtin_schemas() {
            let read = s.get_document_schema(&builtin.id).unwrap();
            assert_eq!(read, builtin, "seeded row deep-equals its builtin_schemas() source");
        }
    }

    /// The parity net between the old hardcoded functions and the seeds.
    #[test]
    fn builtin_schemas_reproduce_todays_pricing_and_total_shape() {
        for b in builtin_schemas() {
            let line_items: Vec<_> =
                b.sections.iter().filter(|s| s.kind == "line_items").collect();
            assert_eq!(line_items.len(), 1, "{}: exactly one line_items section", b.kind);
            assert_eq!(
                line_items[0].priced,
                is_pricing_kind(&b.kind),
                "{}: priced mirrors is_pricing_kind",
                b.kind
            );
            let (total_kind, total_label_key) = total_shape(&b.kind);
            assert_eq!(b.total_kind, total_kind, "{}: total_kind mirrors total_shape", b.kind);
            assert_eq!(
                b.total_label_key, total_label_key,
                "{}: total_label_key mirrors total_shape",
                b.kind
            );
            assert!(
                line_items[0].fields.is_empty()
                    && b.sections.iter().all(|s| s.fields.is_empty()),
                "{}: built-ins carry ZERO fields (launch-safety: zero fill calls)",
                b.kind
            );
        }
    }

    /// WE-A core — the guard exercised the way every real launch exercises it.
    #[test]
    fn tombstoned_builtin_survives_a_fresh_seed_call() {
        let s = Store::open_in_memory("device-a").unwrap();
        s.conn
            .execute(
                "UPDATE document_schemas SET deleted_at = 500, updated_at = 500 WHERE id = ?1",
                [BUILTIN_SCHEMA_ID_ESTIMATE],
            )
            .unwrap();
        seed_builtin_schemas(&s.conn).unwrap();
        let deleted_at: Option<i64> = s
            .conn
            .query_row(
                "SELECT deleted_at FROM document_schemas WHERE id = ?1",
                [BUILTIN_SCHEMA_ID_ESTIMATE],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(deleted_at, Some(500), "stays tombstoned — never resurrected");
        let count: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM document_schemas", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 7, "no duplicate row was inserted either");
    }

    // ---- Stage 2: CRUD + resolution -------------------------------------

    #[test]
    fn list_filters_by_trade_and_includes_null_trade_and_hides_tombstones() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        s.save_document_schema(&custom_schema("custom-hoa", "hoa_addendum", Some("landscape")))
            .unwrap();
        s.save_document_schema(&custom_schema("custom-prop", "unit_turn", Some("property")))
            .unwrap();
        s.remove_document_schema(BUILTIN_SCHEMA_ID_WORK_ORDER).unwrap();

        let landscape: Vec<String> = s
            .list_document_schemas(Some("landscape"))
            .unwrap()
            .into_iter()
            .map(|d| d.kind)
            .collect();
        assert_eq!(
            landscape,
            vec!["estimate", "invoice", "report", "hoa_addendum"],
            "trade ∈ {{landscape, NULL}}, live only (work_order tombstoned), id order \
             (built-ins first)"
        );

        let all = s.list_document_schemas(None).unwrap();
        assert_eq!(all.len(), 8, "None → every live schema (6 built-ins + 2 customs)");
    }

    #[test]
    fn save_upserts_by_id_and_bumps_updated_at_preserving_created_at() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let saved =
            s.save_document_schema(&custom_schema("custom-1", "hoa_addendum", Some("landscape")))
                .unwrap();
        assert_eq!(saved.created_at, 1000);
        assert_eq!(saved.updated_at, 1000);
        assert_eq!(saved.device_id, "device-a", "insert stamps this store's device_id");

        let s = s.with_clock(Arc::new(|| 2000));
        let mut edited = saved.clone();
        edited.label = "HOA Addendum".to_string();
        let resaved = s.save_document_schema(&edited).unwrap();
        assert_eq!(resaved.label, "HOA Addendum");
        assert_eq!(resaved.created_at, 1000, "created_at preserved on upsert");
        assert_eq!(resaved.updated_at, 2000, "updated_at bumped");
        assert_eq!(s.list_document_schemas(None).unwrap().len(), 8, "upsert, not a second row");

        // An empty id mints a fresh UUIDv7 (the FFI "create" convenience).
        let minted = s.save_document_schema(&custom_schema("", "punch_list", Some("landscape")))
            .unwrap();
        assert!(!minted.id.is_empty());
        assert_ne!(minted.id, "custom-1");
    }

    #[test]
    fn remove_tombstones_then_second_remove_is_not_found() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        s.save_document_schema(&custom_schema("custom-1", "hoa_addendum", Some("landscape")))
            .unwrap();
        s.remove_document_schema("custom-1").unwrap();
        assert!(matches!(
            s.get_document_schema("custom-1"),
            Err(CoreError::NotFound { entity: "document_schema", .. })
        ));
        assert!(matches!(
            s.remove_document_schema("custom-1"),
            Err(CoreError::NotFound { entity: "document_schema", .. })
        ));
    }

    #[test]
    fn resolve_prefers_newest_and_matches_kind_plus_trade() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        // The built-in resolves before any custom exists.
        let resolved = s.resolve_active_schema("estimate", Some("landscape")).unwrap().unwrap();
        assert_eq!(resolved.id, BUILTIN_SCHEMA_ID_ESTIMATE);

        // A custom estimate (newer updated_at than the built-in's fixed 0) wins.
        s.save_document_schema(&custom_schema("custom-est", "estimate", Some("landscape")))
            .unwrap();
        let resolved = s.resolve_active_schema("estimate", Some("landscape")).unwrap().unwrap();
        assert_eq!(resolved.id, "custom-est", "newest updated_at wins");

        // Trade must match: the landscape custom does not resolve for property.
        assert!(s.resolve_active_schema("estimate", Some("property")).unwrap().is_none());
        // Kind must match.
        assert!(s.resolve_active_schema("hoa_addendum", Some("landscape")).unwrap().is_none());
        assert!(!s.has_active_schema("hoa_addendum", Some("landscape")).unwrap());
        assert!(s.has_active_schema("estimate", Some("landscape")).unwrap());
    }

    /// Parity guard: `report` (trade NULL) resolves ONLY for a None-template
    /// session — it stays illegal on a landscape session, matching today.
    #[test]
    fn resolve_report_only_for_none_template_not_for_landscape() {
        let s = Store::open_in_memory("device-a").unwrap();
        let resolved = s.resolve_active_schema("report", None).unwrap().unwrap();
        assert_eq!(resolved.id, BUILTIN_SCHEMA_ID_REPORT);
        assert!(s.resolve_active_schema("report", Some("landscape")).unwrap().is_none());
    }

    /// The resurrection consequence: a tombstoned built-in does not resolve —
    /// the build fails truthfully instead of falling back to a hardcoded shape.
    #[test]
    fn resolve_returns_none_for_a_tombstoned_builtin() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        s.remove_document_schema(BUILTIN_SCHEMA_ID_ESTIMATE).unwrap();
        assert!(s.resolve_active_schema("estimate", Some("landscape")).unwrap().is_none());
        assert!(!s.has_active_schema("estimate", Some("landscape")).unwrap());
    }

    /// WE-A end-to-end (§6, hand-recomputed): tombstone a built-in, REOPEN
    /// the store (re-running the seed), simulate the app-update that adds
    /// built-in …0008 punch_list through the very same seed mechanism, and
    /// assert the exact surviving live set.
    #[test]
    fn we_a_reopen_over_a_tombstoned_builtin_yields_the_pinned_surviving_set() {
        let dir = std::env::temp_dir().join(format!("murmur-core-we-a-{}", crate::ids::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("murmur.db");

        // 1. First open seeds {0001..0007} live.
        {
            let s = Store::open(&path, "device-a").unwrap().with_clock(Arc::new(|| 1000));
            let landscape: Vec<String> = s
                .list_document_schemas(Some("landscape"))
                .unwrap()
                .into_iter()
                .map(|d| d.kind)
                .collect();
            assert_eq!(landscape, vec!["estimate", "invoice", "work_order", "report"]);
            // 2. The operator deletes the estimate built-in.
            s.remove_document_schema(BUILTIN_SCHEMA_ID_ESTIMATE).unwrap();
        }

        // 3. Next open re-runs the seed; the app-update also ships built-in
        //    0008 punch_list (landscape, prefix PUN) — same mechanism.
        let s = Store::open(&path, "device-a").unwrap();
        let mut punch = custom_schema(
            "00000000-0000-7000-8000-000000000008",
            "punch_list",
            Some("landscape"),
        );
        punch.number_prefix = "PUN".to_string();
        punch.device_id = crate::domain::BUILTIN_SCHEMA_DEVICE_ID.to_string();
        seed_schemas(&s.conn, &[punch]).unwrap();

        // 4. The pinned surviving set: estimate ABSENT, punch_list present.
        let landscape: Vec<(String, String)> = s
            .list_document_schemas(Some("landscape"))
            .unwrap()
            .into_iter()
            .map(|d| (d.id, d.kind))
            .collect();
        assert_eq!(
            landscape,
            vec![
                (BUILTIN_SCHEMA_ID_INVOICE.to_string(), "invoice".to_string()),
                (BUILTIN_SCHEMA_ID_WORK_ORDER.to_string(), "work_order".to_string()),
                (BUILTIN_SCHEMA_ID_REPORT.to_string(), "report".to_string()),
                ("00000000-0000-7000-8000-000000000008".to_string(), "punch_list".to_string()),
            ],
            "WE-A: [0002 invoice, 0003 work_order, 0007 report, 0008 punch_list], estimate ABSENT"
        );
        let all_live: Vec<String> =
            s.list_document_schemas(None).unwrap().into_iter().map(|d| d.id).collect();
        assert_eq!(all_live.len(), 7, "live set {{0002..0008}}; 0001 remains tombstoned");
        assert!(!all_live.iter().any(|id| id == BUILTIN_SCHEMA_ID_ESTIMATE));
        std::fs::remove_dir_all(dir).ok();
    }

    // ---- Stage 3: save-time validation (R6, reject-never-coerce) ---------

    fn schema_count(s: &Store) -> usize {
        s.list_document_schemas(None).unwrap().len()
    }

    #[test]
    fn reject_unknown_section_kind_nothing_persisted() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let before = schema_count(&s);
        let mut bad = custom_schema("custom-1", "hoa_addendum", Some("landscape"));
        bad.sections.push(SchemaSection {
            key: "gallery".into(),
            kind: "gallery".into(),
            label: "Gallery".into(),
            priced: false,
            fields: vec![],
        });
        let err = s.save_document_schema(&bad).unwrap_err();
        assert!(
            err.to_string()
                .contains("invalid section kind 'gallery'; must be one of: line_items, static, filled"),
            "exact allowlist message, got: {err}"
        );
        assert_eq!(schema_count(&s), before, "nothing persisted on rejection");
        assert!(s.get_document_schema("custom-1").is_err());
    }

    /// WE-D core (§6): the exact error and the unchanged count.
    #[test]
    fn reject_unknown_field_kind_nothing_persisted() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let before = schema_count(&s);
        let mut bad = custom_schema("custom-1", "hoa_addendum", Some("landscape"));
        bad.sections.push(SchemaSection {
            key: "s2".into(),
            kind: "filled".into(),
            label: "S2".into(),
            priced: false,
            fields: vec![SchemaField {
                key: "b".into(),
                kind: "barcode".into(),
                label: "B".into(),
                fill: "walk".into(),
                static_value: None,
            }],
        });
        let err = s.save_document_schema(&bad).unwrap_err();
        assert!(
            err.to_string().contains(
                "invalid field kind 'barcode'; must be one of: line_items, text, long_text, \
                 currency, quantity, date, static"
            ),
            "WE-D's exact message, got: {err}"
        );
        assert_eq!(schema_count(&s), before, "the INSERT is never reached (WE-D)");
    }

    #[test]
    fn reject_unknown_fill() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let mut bad = custom_schema("custom-1", "hoa_addendum", Some("landscape"));
        bad.sections.push(SchemaSection {
            key: "s2".into(),
            kind: "filled".into(),
            label: "S2".into(),
            priced: false,
            fields: vec![SchemaField {
                key: "f".into(),
                kind: "text".into(),
                label: "F".into(),
                fill: "psychic".into(),
                static_value: None,
            }],
        });
        let err = s.save_document_schema(&bad).unwrap_err();
        assert!(
            err.to_string().contains("invalid fill 'psychic'; must be one of: walk, manual, static"),
            "got: {err}"
        );
    }

    #[test]
    fn reject_zero_line_items_sections() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let mut bad = custom_schema("custom-1", "hoa_addendum", Some("landscape"));
        bad.sections.clear();
        let err = s.save_document_schema(&bad).unwrap_err();
        assert!(err.to_string().contains("exactly one line_items section (found 0)"), "got: {err}");
        // Empty kind/label/prefix also reject.
        let mut empty_kind = custom_schema("custom-2", " ", Some("landscape"));
        empty_kind.kind = " ".into();
        assert!(s.save_document_schema(&empty_kind).is_err());
        let mut empty_prefix = custom_schema("custom-3", "hoa_addendum", Some("landscape"));
        empty_prefix.number_prefix = "".into();
        assert!(s.save_document_schema(&empty_prefix).is_err());
    }

    #[test]
    fn reject_two_line_items_sections() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let mut bad = custom_schema("custom-1", "hoa_addendum", Some("landscape"));
        let second = bad.sections[0].clone();
        bad.sections.push(second);
        let err = s.save_document_schema(&bad).unwrap_err();
        assert!(err.to_string().contains("exactly one line_items section (found 2)"), "got: {err}");
        assert!(s.get_document_schema("custom-1").is_err(), "nothing persisted");
    }

    #[test]
    fn valid_custom_schema_saves_and_round_trips() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let mut schema = custom_schema("custom-hoa", "hoa_addendum", Some("landscape"));
        schema.sections.push(SchemaSection {
            key: "approvals".into(),
            kind: "filled".into(),
            label: "Approvals".into(),
            priced: false,
            fields: vec![SchemaField {
                key: "hoa_no".into(),
                kind: "text".into(),
                label: "HOA approval #".into(),
                fill: "walk".into(),
                static_value: None,
            }],
        });
        schema.sections.push(SchemaSection {
            key: "terms".into(),
            kind: "static".into(),
            label: "Terms".into(),
            priced: false,
            fields: vec![SchemaField {
                key: "terms_body".into(),
                kind: "static".into(),
                label: "Terms".into(),
                fill: "static".into(),
                static_value: Some("Valid for 30 days.".into()),
            }],
        });
        let saved = s.save_document_schema(&schema).unwrap();
        let read = s.get_document_schema("custom-hoa").unwrap();
        assert_eq!(read, saved);
        assert_eq!(read.sections, schema.sections, "sections round-trip through the envelope");
        assert_eq!(read.sections[2].fields[0].static_value.as_deref(), Some("Valid for 30 days."));
    }
}
