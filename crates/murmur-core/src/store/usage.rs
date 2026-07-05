use harness::Usage;
use rusqlite::Row;

use crate::domain::LlmUsageRow;
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

fn usage_from_row(row: &Row) -> Result<LlmUsageRow, CoreError> {
    Ok(LlmUsageRow {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        session_id: row.get("session_id").map_err(CoreError::Sqlite)?,
        purpose: row.get("purpose").map_err(CoreError::Sqlite)?,
        input_tokens: row.get::<_, i64>("input_tokens").map_err(CoreError::Sqlite)? as u64,
        output_tokens: row.get::<_, i64>("output_tokens").map_err(CoreError::Sqlite)? as u64,
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
    })
}

impl Store {
    /// Logs one LLM call's token cost (R9). `session_id` is None for
    /// session-independent work (reflection).
    pub fn record_llm_usage(
        &self,
        session_id: Option<&str>,
        purpose: &str,
        usage: &Usage,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO llm_usage (id, session_id, purpose, input_tokens, output_tokens, created_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                new_id(),
                session_id,
                purpose,
                usage.input_tokens as i64,
                usage.output_tokens as i64,
                self.now() as i64,
                self.device_id,
            ],
        )?;
        Ok(())
    }

    pub fn list_llm_usage_for_session(&self, session_id: &str) -> Result<Vec<LlmUsageRow>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, purpose, input_tokens, output_tokens, created_at, device_id
             FROM llm_usage WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let mut rows = stmt.query([session_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(usage_from_row(row)?);
        }
        Ok(out)
    }

    /// (total input tokens, total output tokens) across all recorded calls —
    /// the spend meter's raw feed.
    pub fn usage_totals(&self) -> Result<(u64, u64), CoreError> {
        let (i, o): (i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0) FROM llm_usage",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((i as u64, o as u64))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harness::Usage;

    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000))
    }

    #[test]
    fn record_and_list_for_session() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.record_llm_usage(
            Some(&session.id),
            "processing",
            &Usage { input_tokens: 900, output_tokens: 120 },
        )
        .unwrap();
        let rows = s.list_llm_usage_for_session(&session.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].purpose, "processing");
        assert_eq!(rows[0].input_tokens, 900);
        assert_eq!(rows[0].output_tokens, 120);
        assert_eq!(rows[0].created_at, 1000);
    }

    #[test]
    fn sessionless_usage_is_allowed() {
        let s = store();
        s.record_llm_usage(None, "reflection", &Usage { input_tokens: 300, output_tokens: 80 })
            .unwrap();
        assert_eq!(s.usage_totals().unwrap(), (300, 80));
    }

    #[test]
    fn totals_sum_across_rows() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.record_llm_usage(Some(&session.id), "processing", &Usage { input_tokens: 10, output_tokens: 1 })
            .unwrap();
        s.record_llm_usage(None, "reflection", &Usage { input_tokens: 5, output_tokens: 2 })
            .unwrap();
        assert_eq!(s.usage_totals().unwrap(), (15, 3));
        assert_eq!(s.list_llm_usage_for_session(&session.id).unwrap().len(), 1);
    }

    #[test]
    fn unknown_session_is_rejected() {
        let s = store();
        let err = s.record_llm_usage(Some("nope"), "processing", &Usage::default());
        assert!(err.is_err(), "FK to sessions must hold");
    }
}
