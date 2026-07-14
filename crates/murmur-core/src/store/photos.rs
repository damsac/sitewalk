use std::collections::HashMap;

use rusqlite::Row;

use crate::domain::Photo;
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

const PHOTO_COLS: &str =
    "id, session_id, item_id, filename, captured_at, created_at, updated_at, device_id";

fn photo_from_row(row: &Row) -> Result<Photo, CoreError> {
    Ok(Photo {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        session_id: row.get("session_id").map_err(CoreError::Sqlite)?,
        item_id: row.get("item_id").map_err(CoreError::Sqlite)?,
        filename: row.get("filename").map_err(CoreError::Sqlite)?,
        captured_at: row.get::<_, i64>("captured_at").map_err(CoreError::Sqlite)? as u64,
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        updated_at: row.get::<_, i64>("updated_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
    })
}

impl Store {
    /// Attaches a photo to a session, optionally to a specific captured item.
    /// `session_id` must be a live session (`NotFound` otherwise); if `item_id`
    /// is given, it must be a live item belonging to THIS session (`InvalidState`
    /// otherwise — membership check, Plan 11 D2/acceptance criterion 3).
    /// `captured_at` defaults to `self.now()` when `None`.
    pub fn add_photo(
        &self,
        session_id: &str,
        item_id: Option<&str>,
        filename: &str,
        captured_at: Option<u64>,
    ) -> Result<Photo, CoreError> {
        self.get_session(session_id)?; // NotFound if missing/tombstoned

        if let Some(item_id) = item_id {
            let item = self.get_item(item_id)?; // NotFound if missing/tombstoned
            if item.session_id != session_id {
                return Err(CoreError::InvalidState(format!(
                    "item {item_id} does not belong to session {session_id}"
                )));
            }
        }

        let now = self.now();
        let photo = Photo {
            id: new_id(),
            session_id: session_id.to_string(),
            item_id: item_id.map(str::to_string),
            filename: filename.to_string(),
            captured_at: captured_at.unwrap_or(now),
            created_at: now,
            updated_at: now,
            device_id: self.device_id.clone(),
        };
        self.conn.execute(
            "INSERT INTO photos (id, session_id, item_id, filename, captured_at, created_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                photo.id,
                photo.session_id,
                photo.item_id,
                photo.filename,
                photo.captured_at as i64,
                photo.created_at as i64,
                photo.updated_at as i64,
                photo.device_id,
            ],
        )?;
        Ok(photo)
    }

    pub fn get_photo(&self, id: &str) -> Result<Photo, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PHOTO_COLS} FROM photos WHERE id = ?1 AND deleted_at IS NULL"
        ))?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => photo_from_row(row),
            None => Err(CoreError::NotFound { entity: "photo", id: id.to_string() }),
        }
    }

    /// Photos of one session in insertion order (UUIDv7 ids sort by creation).
    pub fn list_photos_for_session(&self, session_id: &str) -> Result<Vec<Photo>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PHOTO_COLS} FROM photos
             WHERE session_id = ?1 AND deleted_at IS NULL ORDER BY id ASC"
        ))?;
        let mut rows = stmt.query([session_id])?;
        let mut photos = Vec::new();
        while let Some(row) = rows.next()? {
            photos.push(photo_from_row(row)?);
        }
        Ok(photos)
    }

    pub fn remove_photo(&self, id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        let changed = self.conn.execute(
            "UPDATE photos SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, id],
        )?;
        if changed == 0 {
            return Err(CoreError::NotFound { entity: "photo", id: id.to_string() });
        }
        Ok(())
    }

    /// Core's entire file contract (Plan 11 D4): all live filenames, across all
    /// sessions. The shell's reconciling sweep deletes any file on disk not in
    /// this set.
    pub fn list_live_photo_filenames(&self) -> Result<Vec<String>, CoreError> {
        let mut stmt = self.conn.prepare("SELECT filename FROM photos WHERE deleted_at IS NULL")?;
        let mut rows = stmt.query([])?;
        let mut names = Vec::new();
        while let Some(row) = rows.next()? {
            names.push(row.get::<_, String>(0)?);
        }
        Ok(names)
    }

    pub fn count_photos_for_session(&self, session_id: &str) -> Result<i64, CoreError> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM photos WHERE session_id = ?1 AND deleted_at IS NULL",
                [session_id],
                |r| r.get(0),
            )
            .map_err(CoreError::Sqlite)
    }

    /// Batched per-item live-photo counts for a session (photo_count
    /// fast-follow, Plan 11 D6): one `GROUP BY item_id` query, never an N+1
    /// per board item. Only photos attached to a specific item are counted
    /// (`item_id IS NOT NULL`); session-level photos don't count for any item.
    /// An item with no live photos is simply ABSENT from the map — callers
    /// treat a missing key as `0` (see `convert::board_item`).
    pub fn count_live_photos_by_item_for_session(
        &self,
        session_id: &str,
    ) -> Result<HashMap<String, u32>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT item_id, COUNT(*) FROM photos
             WHERE session_id = ?1 AND deleted_at IS NULL AND item_id IS NOT NULL
             GROUP BY item_id",
        )?;
        let mut rows = stmt.query([session_id])?;
        let mut counts = HashMap::new();
        while let Some(row) = rows.next()? {
            let item_id: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            counts.insert(item_id, count as u32);
        }
        Ok(counts)
    }

    /// Demote live photos whose `item_id` references a now-tombstoned item in
    /// `session_id` to session-level (`item_id := NULL`). Runs INSIDE the
    /// caller's transaction, AFTER the item tombstone. Order-independent: it
    /// keys off `deleted_at IS NOT NULL` on the item, so it demotes exactly the
    /// items just swept (and any earlier-swept item, already idempotent). Plan
    /// 11 D3 — a live photo must never reference a tombstoned item.
    pub(crate) fn demote_photos_of_tombstoned_items(&self, session_id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        self.conn.execute(
            "UPDATE photos SET item_id = NULL, updated_at = ?1
             WHERE session_id = ?2 AND deleted_at IS NULL AND item_id IS NOT NULL
               AND item_id IN (SELECT id FROM items
                               WHERE session_id = ?2 AND deleted_at IS NOT NULL)",
            rusqlite::params![now, session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::error::CoreError;
    use crate::store::Store;

    fn store_with_session() -> (Store, String) {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 1000));
        let session = s.start_session(None).unwrap();
        (s, session.id)
    }

    #[test]
    fn migrates_to_v5() {
        let s = Store::open_in_memory("device-a").unwrap();
        let v: i64 = s.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert!(v >= 5, "photos migration bumps user_version to at least 5 (v6 added items.right_text, Plan 16)");
    }

    #[test]
    fn add_list_get_photo_session_level() {
        let (s, sid) = store_with_session();
        let p = s.add_photo(&sid, None, "a1b2.jpg", Some(1234)).unwrap();
        assert_eq!(p.session_id, sid);
        assert_eq!(p.item_id, None);
        assert_eq!(p.filename, "a1b2.jpg");
        assert_eq!(p.captured_at, 1234);
        assert_eq!(p.created_at, 1000); // injected clock
        assert_eq!(s.list_photos_for_session(&sid).unwrap(), vec![p.clone()]);
        assert_eq!(s.get_photo(&p.id).unwrap(), p);
    }

    #[test]
    fn add_photo_captured_at_defaults_to_now() {
        let (s, sid) = store_with_session();
        let p = s.add_photo(&sid, None, "x.jpg", None).unwrap();
        assert_eq!(p.captured_at, 1000, "None captured_at stamps now()");
    }

    #[test]
    fn add_photo_to_item_validates_membership() {
        let (s, sid) = store_with_session();
        let item = s.add_item(&sid, "todo", "deck").unwrap();
        let p = s.add_photo(&sid, Some(&item.id), "d.jpg", None).unwrap();
        assert_eq!(p.item_id.as_deref(), Some(item.id.as_str()));
        // item that isn't in this session → error (InvalidState)
        let other = s.start_session(None).unwrap();
        let other_item = s.add_item(&other.id, "todo", "x").unwrap();
        assert!(matches!(
            s.add_photo(&sid, Some(&other_item.id), "e.jpg", None),
            Err(CoreError::InvalidState(_))
        ));
    }

    #[test]
    fn add_photo_to_missing_session_is_not_found() {
        let (s, _) = store_with_session();
        assert!(matches!(
            s.add_photo("nope", None, "z.jpg", None),
            Err(CoreError::NotFound { entity: "session", .. })
        ));
    }

    #[test]
    fn live_filename_uniqueness_is_enforced() {
        let (s, sid) = store_with_session();
        s.add_photo(&sid, None, "dup.jpg", None).unwrap();
        assert!(s.add_photo(&sid, None, "dup.jpg", None).is_err(), "no two live rows share a filename");
        // after tombstone, the name frees up
        let again = s.list_photos_for_session(&sid).unwrap()[0].id.clone();
        s.remove_photo(&again).unwrap();
        assert!(s.add_photo(&sid, None, "dup.jpg", None).is_ok(), "tombstoned filename can be reused");
    }

    #[test]
    fn remove_photo_is_a_tombstone() {
        let (s, sid) = store_with_session();
        let p = s.add_photo(&sid, None, "a.jpg", None).unwrap();
        s.remove_photo(&p.id).unwrap();
        assert!(s.list_photos_for_session(&sid).unwrap().is_empty());
        assert!(matches!(s.remove_photo(&p.id), Err(CoreError::NotFound { .. })));
        // raw row survives (tombstone, not erase)
        let raw: i64 = s.conn.query_row("SELECT COUNT(*) FROM photos WHERE id=?1", [&p.id], |r| r.get(0)).unwrap();
        assert_eq!(raw, 1);
    }

    #[test]
    fn list_live_photo_filenames_spans_sessions_and_skips_tombstoned() {
        let (s, sid_a) = store_with_session();
        let sid_b = s.start_session(None).unwrap().id;
        s.add_photo(&sid_a, None, "a.jpg", None).unwrap();
        let gone = s.add_photo(&sid_b, None, "b.jpg", None).unwrap();
        s.add_photo(&sid_b, None, "c.jpg", None).unwrap();
        s.remove_photo(&gone.id).unwrap();
        let mut names = s.list_live_photo_filenames().unwrap();
        names.sort();
        assert_eq!(names, vec!["a.jpg".to_string(), "c.jpg".to_string()], "b.jpg tombstoned → excluded");
    }

    #[test]
    fn count_photos_for_session_counts_live_only() {
        let (s, sid) = store_with_session();
        s.add_photo(&sid, None, "a.jpg", None).unwrap();
        let g = s.add_photo(&sid, None, "b.jpg", None).unwrap();
        s.remove_photo(&g.id).unwrap();
        assert_eq!(s.count_photos_for_session(&sid).unwrap(), 1);
    }

    #[test]
    fn count_live_photos_by_item_for_session_batches_and_skips_session_level() {
        let (s, sid) = store_with_session();
        let i1 = s.add_item(&sid, "todo", "I1").unwrap();
        let i2 = s.add_item(&sid, "todo", "I2").unwrap();
        s.add_photo(&sid, Some(&i1.id), "a.jpg", None).unwrap();
        s.add_photo(&sid, Some(&i1.id), "b.jpg", None).unwrap();
        s.add_photo(&sid, Some(&i2.id), "c.jpg", None).unwrap();
        s.add_photo(&sid, None, "d.jpg", None).unwrap(); // session-level, not counted
        let gone = s.add_photo(&sid, Some(&i2.id), "e.jpg", None).unwrap();
        s.remove_photo(&gone.id).unwrap(); // tombstoned, not counted

        let counts = s.count_live_photos_by_item_for_session(&sid).unwrap();
        assert_eq!(counts.get(i1.id.as_str()), Some(&2));
        assert_eq!(counts.get(i2.id.as_str()), Some(&1));
        assert_eq!(counts.len(), 2, "items with zero live photos are absent, not zero-valued");
    }

    #[test]
    fn delete_session_cascades_to_photos() {
        // Session S; items I1,I2; P1 session-level, P2 attached to I1.
        let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 5000));
        let sid = s.start_session(None).unwrap().id;
        let i1 = s.add_item(&sid, "todo", "I1").unwrap();
        let _i2 = s.add_item(&sid, "todo", "I2").unwrap();
        let p1 = s.add_photo(&sid, None, "p1.jpg", None).unwrap();
        let p2 = s.add_photo(&sid, Some(&i1.id), "p2.jpg", None).unwrap();

        s.delete_session(&sid).unwrap();

        // Both photos tombstoned in the same op; nothing readable, raw rows survive.
        assert!(s.list_photos_for_session(&sid).unwrap().is_empty());
        for id in [&p1.id, &p2.id] {
            let raw: i64 = s.conn.query_row("SELECT COUNT(*) FROM photos WHERE id=?1", [id], |r| r.get(0)).unwrap();
            assert_eq!(raw, 1, "tombstone, not erase");
            let del: i64 = s.conn.query_row("SELECT deleted_at FROM photos WHERE id=?1", [id], |r| r.get(0)).unwrap();
            assert_eq!(del, 5000);
        }
        // filenames leave the live set → the shell sweep will reap the bytes.
        assert!(s.list_live_photo_filenames().unwrap().is_empty());
    }

    #[test]
    fn finish_swap_demotes_photos_of_swept_items_and_never_loses_them() {
        use crate::domain::ItemSource;
        let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 6000));
        let sid = s.start_session(None).unwrap().id;
        // Live board: I1 (live). Photos: P1 session-level, P2 attached to I1.
        let i1 = s.add_item_with_source(&sid, "todo", "live", ItemSource::Live).unwrap();
        let p1 = s.add_photo(&sid, None, "p1.jpg", None).unwrap();
        let p2 = s.add_photo(&sid, Some(&i1.id), "p2.jpg", None).unwrap();
        // This run extracts one authoritative item A1.
        let a1 = s.add_item_with_source(&sid, "todo", "auth", ItemSource::Authoritative).unwrap();
        s.end_session(&sid).unwrap();
        s.finish_session_processed(&sid, "done", &harness::Usage::default(), std::slice::from_ref(&a1.id)).unwrap();

        // I1 swept (deleted_at=6000). A1 survives. Both photos survive at session scope.
        let items: Vec<String> = s.list_items_for_session(&sid).unwrap().into_iter().map(|i| i.id).collect();
        assert_eq!(items, vec![a1.id.clone()], "I1 swept, A1 remains");
        let photos = s.list_photos_for_session(&sid).unwrap();
        let by_id = |pid: &str| photos.iter().find(|p| p.id == pid).unwrap();
        assert_eq!(photos.len(), 2, "no photo lost in the swap");
        assert_eq!(by_id(&p1.id).item_id, None, "session-level P1 untouched");
        // THE FIX: P2's item_id was I1 (now tombstoned) → demoted to NULL, updated_at bumped.
        assert_eq!(by_id(&p2.id).item_id, None, "P2 demoted to session-level, not left dangling");
        assert_eq!(by_id(&p2.id).updated_at, 6000, "demotion bumps updated_at (sync-visible)");
        // Neither photo references a tombstoned item.
        assert!(photos.iter().all(|p| p.item_id.is_none()));
    }

    #[test]
    fn finish_swap_keeps_photos_on_surviving_manual_and_this_run_items() {
        use crate::domain::ItemSource;
        let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 7000));
        let sid = s.start_session(None).unwrap().id;
        let manual = s.add_item_with_source(&sid, "note", "manual", ItemSource::Manual).unwrap();
        let a1 = s.add_item_with_source(&sid, "todo", "auth", ItemSource::Authoritative).unwrap();
        let pm = s.add_photo(&sid, Some(&manual.id), "pm.jpg", None).unwrap();
        let pa = s.add_photo(&sid, Some(&a1.id), "pa.jpg", None).unwrap();
        s.end_session(&sid).unwrap();
        // manual survives (never swept); a1 is in run_item_ids (survives) → both linkages kept.
        s.finish_session_processed(&sid, "done", &harness::Usage::default(), std::slice::from_ref(&a1.id)).unwrap();
        let photos = s.list_photos_for_session(&sid).unwrap();
        assert_eq!(photos.iter().find(|p| p.id == pm.id).unwrap().item_id.as_deref(), Some(manual.id.as_str()));
        assert_eq!(photos.iter().find(|p| p.id == pa.id).unwrap().item_id.as_deref(), Some(a1.id.as_str()));
    }

    #[test]
    fn clear_authoritative_outputs_demotes_photos_of_swept_authoritative_items() {
        use crate::domain::ItemSource;
        let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 8000));
        let sid = s.start_session(None).unwrap().id;
        let stale = s.add_item_with_source(&sid, "todo", "stale auth", ItemSource::Authoritative).unwrap();
        let p = s.add_photo(&sid, Some(&stale.id), "p.jpg", None).unwrap();
        s.clear_authoritative_outputs(&sid).unwrap();
        // stale auth item tombstoned; its photo demoted, not lost.
        assert!(s.list_items_for_session(&sid).unwrap().is_empty());
        let got = s.get_photo(&p.id).unwrap();
        assert_eq!(got.item_id, None, "demoted");
        assert_eq!(got.updated_at, 8000);
    }

    #[test]
    fn delete_item_demotes_its_photos_not_deletes_them() {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 9000));
        let sid = s.start_session(None).unwrap().id;
        let item = s.add_item(&sid, "todo", "wrong todo").unwrap();
        let p = s.add_photo(&sid, Some(&item.id), "p.jpg", None).unwrap();
        s.delete_item(&item.id).unwrap();
        // the item is gone but the user's photo survives, demoted to session-level.
        assert!(s.list_items_for_session(&sid).unwrap().is_empty());
        let got = s.get_photo(&p.id).unwrap();
        assert_eq!(got.item_id, None);
        assert_eq!(got.updated_at, 9000);
        assert_eq!(s.count_photos_for_session(&sid).unwrap(), 1);
    }
}
