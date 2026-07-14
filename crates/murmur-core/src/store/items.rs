use rusqlite::Row;

use crate::domain::{CapturedItem, ItemSource, SessionStatus};
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

const ITEM_COLS: &str =
    "id, session_id, kind, text, right_text, source, done, created_at, updated_at, device_id";

fn item_from_row(row: &Row) -> Result<CapturedItem, CoreError> {
    Ok(CapturedItem {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        session_id: row.get("session_id").map_err(CoreError::Sqlite)?,
        kind: row.get("kind").map_err(CoreError::Sqlite)?,
        text: row.get("text").map_err(CoreError::Sqlite)?,
        right: row.get("right_text").map_err(CoreError::Sqlite)?,
        source: {
            let raw: String = row.get("source").map_err(CoreError::Sqlite)?;
            ItemSource::parse(&raw)?
        },
        done: row.get::<_, i64>("done").map_err(CoreError::Sqlite)? != 0,
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        updated_at: row.get::<_, i64>("updated_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
    })
}

impl Store {
    /// Adds an item to a session. Works for agent extraction (Plans 04/05)
    /// and manual entry alike (story 10: manual parity — nothing is agent-only).
    /// A bare add is manual/parity: source=Manual.
    pub fn add_item(&self, session_id: &str, kind: &str, text: &str) -> Result<CapturedItem, CoreError> {
        self.add_item_with_source(session_id, kind, text, ItemSource::Manual)
    }

    /// Adds an item with an explicit source, ungated. The processing pipeline
    /// uses this for `authoritative` writes (it owns the session; no status
    /// gate applies during processing).
    pub fn add_item_with_source(
        &self,
        session_id: &str,
        kind: &str,
        text: &str,
        source: ItemSource,
    ) -> Result<CapturedItem, CoreError> {
        self.get_session(session_id)?; // NotFound if missing/tombstoned
        self.insert_item(session_id, kind, text, source)
    }

    /// Same as `add_item`, but only writes if the session's CURRENT status
    /// matches `required` — the status read and the insert happen against
    /// the same `&self` call with no intervening await, so as long as every
    /// caller shares one `Store` behind a single lock (as `AddItemTool` and
    /// `LiveExtractor` do), no window exists between the check and the
    /// write. Returns `Ok(None)` on a status mismatch (nothing written);
    /// `Ok(Some(item))` on success.
    pub fn add_item_if_status(
        &self,
        session_id: &str,
        kind: &str,
        text: &str,
        required: SessionStatus,
        source: ItemSource,
    ) -> Result<Option<CapturedItem>, CoreError> {
        let session = self.get_session(session_id)?; // NotFound if missing/tombstoned
        if session.status != required {
            return Ok(None);
        }
        self.insert_item(session_id, kind, text, source).map(Some)
    }

    fn insert_item(&self, session_id: &str, kind: &str, text: &str, source: ItemSource)
        -> Result<CapturedItem, CoreError>
    {
        let now = self.now();
        let item = CapturedItem {
            id: new_id(),
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
            right: String::new(),
            source,
            done: false,
            created_at: now,
            updated_at: now,
            device_id: self.device_id.clone(),
        };
        self.conn.execute(
            "INSERT INTO items (id, session_id, kind, text, right_text, source, done, created_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9)",
            rusqlite::params![
                item.id, item.session_id, item.kind, item.text, item.right,
                item.source.as_str(),
                item.created_at as i64, item.updated_at as i64, item.device_id,
            ],
        )?;
        Ok(item)
    }

    /// Partial update of an item's editable fields (Plan 16). Mirrors
    /// `set_item_done`: tombstone-guarded, bumps `updated_at`, preserves
    /// id/created_at/source/done. A `None` field is left unchanged.
    /// Ungated at the store layer (exactly like `set_item_done`/
    /// `delete_item`) — the session-status gate is a boundary concern and
    /// lives in the FFI layer (Plan 16 D1-16/D3-16).
    pub fn update_item(
        &self,
        id: &str,
        text: Option<&str>,
        kind: Option<&str>,
        right: Option<&str>,
    ) -> Result<CapturedItem, CoreError> {
        // COALESCE keeps None fields untouched in one statement — no
        // read-modify-write.
        let changed = self.conn.execute(
            "UPDATE items SET text = COALESCE(?1, text), kind = COALESCE(?2, kind),
                              right_text = COALESCE(?3, right_text), updated_at = ?4
             WHERE id = ?5 AND deleted_at IS NULL",
            rusqlite::params![text, kind, right, self.now() as i64, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "item", id: id.to_string() });
        }
        self.get_item(id)
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
            Some(row) => item_from_row(row),
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
            items.push(item_from_row(row)?);
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
            items.push(item_from_row(row)?);
        }
        Ok(items)
    }

    /// Tombstones an item and demotes its photos to session-level in one
    /// transaction (Plan 11 D3): deleting a wrongly-extracted item must not
    /// destroy a real photo — the photo survives, unlinked.
    pub fn delete_item(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let tx = self.conn.unchecked_transaction()?;
        let changed = self.conn.execute(
            "UPDATE items SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "item", id: id.to_string() });
        }
        self.conn.execute(
            "UPDATE photos SET item_id = NULL, updated_at = ?1 WHERE item_id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::domain::CapturedItem;
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
    fn get_missing_item_is_not_found() {
        let (s, _) = store_with_session();
        assert!(matches!(
            s.get_item("nope"),
            Err(CoreError::NotFound { entity: "item", .. })
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
    fn add_item_if_status_writes_when_status_matches() {
        use crate::domain::ItemSource;
        use crate::domain::SessionStatus;
        let (s, sid) = store_with_session();
        let item = s.add_item_if_status(&sid, "todo", "order lumber", SessionStatus::Recording, ItemSource::Live).unwrap();
        assert!(item.is_some());
        assert_eq!(s.list_items_for_session(&sid).unwrap().len(), 1);
    }

    #[test]
    fn add_item_if_status_no_ops_when_status_mismatches() {
        use crate::domain::ItemSource;
        use crate::domain::SessionStatus;
        let (s, sid) = store_with_session();
        s.end_and_record_session(&sid).unwrap(); // Recording -> AwaitingProcessing
        let item = s.add_item_if_status(&sid, "todo", "order lumber", SessionStatus::Recording, ItemSource::Live).unwrap();
        assert!(item.is_none());
        assert!(s.list_items_for_session(&sid).unwrap().is_empty());
    }

    #[test]
    fn add_item_defaults_to_manual_source() {
        use crate::domain::ItemSource;
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "order lumber").unwrap();
        assert_eq!(item.source, ItemSource::Manual);
        // round-trips through the DB read
        assert_eq!(s.list_items_for_session(&sid).unwrap()[0].source, ItemSource::Manual);
    }

    #[test]
    fn add_item_with_source_persists_the_source() {
        use crate::domain::ItemSource;
        let (s, sid) = store_with_session();
        let live = s.add_item_with_source(&sid, "todo", "live one", ItemSource::Live).unwrap();
        let auth = s.add_item_with_source(&sid, "todo", "auth one", ItemSource::Authoritative).unwrap();
        assert_eq!(live.source, ItemSource::Live);
        assert_eq!(auth.source, ItemSource::Authoritative);
    }

    #[test]
    fn existing_rows_backfill_as_authoritative() {
        // simulate a pre-migration row: raw insert without a source column value
        let (s, sid) = store_with_session();
        s.conn.execute(
            "INSERT INTO items (id, session_id, kind, text, done, created_at, updated_at, device_id)
             VALUES ('legacy', ?1, 'todo', 'old', 0, 1, 1, 'device-a')",
            [&sid],
        ).unwrap();
        let legacy = s.list_items_for_session(&sid).unwrap()
            .into_iter().find(|i| i.id == "legacy").unwrap();
        assert_eq!(legacy.source, crate::domain::ItemSource::Authoritative,
            "the column DEFAULT backfills rows that predate the source column");
    }

    #[test]
    fn open_todos_include_live_items() {
        use crate::domain::ItemSource;
        let (s, sid) = store_with_session();
        s.add_item_with_source(&sid, "todo", "live todo", ItemSource::Live).unwrap();
        // Morning glance surfaces live items too — post-06a they are the safety
        // net for a still-processing / failed session (decision recorded in plan).
        let open: Vec<_> = s.list_open_todos().unwrap().into_iter().map(|i| i.text).collect();
        assert_eq!(open, vec!["live todo".to_string()]);
    }

    // ---- Plan 16: right_text column + update_item -----------------------

    #[test]
    fn fresh_store_is_at_schema_v6() {
        let (s, _) = store_with_session();
        let v: i64 =
            s.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 6, "v6 added items.right_text (Plan 16)");
    }

    #[test]
    fn legacy_rows_without_right_text_read_back_empty_right() {
        // simulate a pre-migration row: raw insert without a right_text value
        let (s, sid) = store_with_session();
        s.conn.execute(
            "INSERT INTO items (id, session_id, kind, text, done, created_at, updated_at, device_id)
             VALUES ('legacy', ?1, 'todo', 'old', 0, 1, 1, 'device-a')",
            [&sid],
        ).unwrap();
        let legacy = s.get_item("legacy").unwrap();
        assert_eq!(legacy.right, "",
            "the column DEFAULT backfills rows that predate right_text");
    }

    #[test]
    fn add_item_yields_empty_right() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "order lumber").unwrap();
        assert_eq!(item.right, "");
        // round-trips through the DB read
        assert_eq!(s.list_items_for_session(&sid).unwrap()[0].right, "");
    }

    #[test]
    fn update_item_sets_text_and_kind_and_preserves_the_rest() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "Power edger").unwrap();
        let s = s.with_clock(Arc::new(|| 2000));
        let updated = s.update_item(&item.id, Some("Mower"), Some("part"), None).unwrap();
        assert_eq!(updated.text, "Mower");
        assert_eq!(updated.kind, "part");
        assert_eq!(updated.right, "");
        assert_eq!(updated.done, item.done, "done preserved");
        assert_eq!(updated.id, item.id, "id preserved");
        assert_eq!(updated.created_at, item.created_at, "created_at preserved");
        assert_eq!(updated.source, item.source, "source preserved");
        assert_eq!(updated.updated_at, 2000, "updated_at bumped");
        // the DB read reflects the same fields, not just the returned echo
        assert_eq!(s.get_item(&item.id).unwrap(), updated);
    }

    #[test]
    fn update_item_right_only_leaves_text_and_kind_alone() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "part", "bark mulch").unwrap();
        let updated = s.update_item(&item.id, None, None, Some("3 CU YD")).unwrap();
        assert_eq!(updated.right, "3 CU YD");
        assert_eq!(updated.text, "bark mulch");
        assert_eq!(updated.kind, "part");
        assert_eq!(s.get_item(&item.id).unwrap().right, "3 CU YD", "right round-trips");
    }

    #[test]
    fn all_none_update_bumps_updated_at_and_changes_nothing_else() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "order lumber").unwrap();
        let s = s.with_clock(Arc::new(|| 3000));
        let updated = s.update_item(&item.id, None, None, None).unwrap();
        assert_eq!(updated.updated_at, 3000);
        assert_eq!(
            CapturedItem { updated_at: item.updated_at, ..updated },
            item,
            "everything except updated_at is unchanged"
        );
    }

    #[test]
    fn update_item_on_tombstoned_id_is_not_found() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "x").unwrap();
        s.delete_item(&item.id).unwrap();
        assert!(matches!(
            s.update_item(&item.id, Some("y"), None, None),
            Err(CoreError::NotFound { entity: "item", .. })
        ));
    }

    #[test]
    fn kind_retag_moves_item_in_and_out_of_open_todos() {
        // The WE-A/WE-C cascade, both directions: a kind edit re-files the
        // item for the morning glance without removing it from the session.
        let (s, sid) = store_with_session();
        let b1 = s.add_item(&sid, "todo", "call supplier").unwrap();
        let b2 = s.add_item(&sid, "todo", "order sod").unwrap();
        let open = |s: &Store| -> Vec<String> {
            s.list_open_todos().unwrap().into_iter().map(|i| i.id).collect()
        };
        assert_eq!(open(&s), vec![b1.id.clone(), b2.id.clone()]);

        s.update_item(&b1.id, None, Some("part"), None).unwrap();
        assert_eq!(open(&s), vec![b2.id.clone()], "todo -> part drops B1 from the glance");
        assert_eq!(s.list_items_for_session(&sid).unwrap().len(), 2,
            "re-filed, not removed — B1 stays in the session");

        s.update_item(&b1.id, None, Some("todo"), None).unwrap();
        assert_eq!(open(&s), vec![b1.id, b2.id], "part -> todo re-adds B1");
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
