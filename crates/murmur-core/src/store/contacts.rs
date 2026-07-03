use rusqlite::Row;

use crate::domain::Contact;
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

const CONTACT_COLS: &str = "id, name, trade, phone, notes, created_at, updated_at, device_id";

fn contact_from_row(row: &Row) -> Result<Contact, CoreError> {
    Ok(Contact {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        name: row.get("name").map_err(CoreError::Sqlite)?,
        trade: row.get("trade").map_err(CoreError::Sqlite)?,
        phone: row.get("phone").map_err(CoreError::Sqlite)?,
        notes: row.get("notes").map_err(CoreError::Sqlite)?,
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        updated_at: row.get::<_, i64>("updated_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
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
            Some(row) => contact_from_row(row),
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
            contacts.push(contact_from_row(row)?);
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
