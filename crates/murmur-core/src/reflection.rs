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

    /// Reflection success exit (the coordinator calls this): records the
    /// reflection signals AND logs the LLM cost in one transaction — a crash
    /// between the two can't leave a recorded reflection with unlogged spend
    /// (or vice versa). Mirrors `finish_session_processed`.
    pub fn finish_reflection(&self, churn: f32, usage: &harness::Usage) -> Result<(), CoreError> {
        let tx = self.conn.unchecked_transaction()?;
        self.record_reflection(churn)?;
        self.record_llm_usage(None, "reflection", usage)?;
        tx.commit()?;
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
             -- >= not >: last_reflected_at is stamped AFTER the reflection runs, so a
             -- session ending between the activity query and the stamp would land exactly
             -- ON the boundary and be orphaned forever by >; >= can at worst re-include
             -- a boundary session once (harmless), never lose one.
             WHERE s.deleted_at IS NULL AND s.ended_at IS NOT NULL AND s.ended_at >= ?1
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
    fn finish_reflection_records_signals_and_logs_cost() {
        let s = store();
        s.record_session_completed().unwrap();
        s.finish_reflection(0.3, &harness::Usage { input_tokens: 200, output_tokens: 40 })
            .unwrap();
        let signals = s.reflection_signals().unwrap();
        assert_eq!(signals.completed_reflections, 1);
        assert_eq!(signals.sessions_since_reflection, 0);
        assert_eq!(signals.recent_churn, vec![0.3]);
        assert_eq!(s.last_reflected_at().unwrap(), 1000);
        assert_eq!(s.usage_totals().unwrap(), (200, 40), "reflection cost logged");
        let purpose: String = s
            .conn
            .query_row("SELECT purpose FROM llm_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(purpose, "reflection");
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
    fn activity_includes_session_ending_exactly_at_last_reflection() {
        let s = store().with_clock(Arc::new(|| 2000));
        s.record_reflection(0.1).unwrap();
        // a session that starts AND ends at the same second the reflection was
        // stamped must not be orphaned by the boundary comparison
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "boundary session words").unwrap();
        s.end_session(&a.id).unwrap();

        let activity = s.activity_for_reflection(10).unwrap();
        assert_eq!(activity.len(), 1);
        assert!(activity[0].contains("boundary session words"));
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
