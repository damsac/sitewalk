//! Job CRUD across UniFFI — the seam the jobs board builds on.
//!
//! Jobs have existed in `murmur-core` since v1 (`store/jobs.rs`, and
//! `sessions.job_id` has always been a foreign key to them), but nothing was
//! ever exposed to the app, so the board stayed session-flat. This module is
//! the missing surface, not a new model.
//!
//! Engine-keyed, mirroring the schema/vocabulary CRUD discipline: lock →
//! mutate → return a typed `EngineError`, never a panic (Plan 07 CANON).
//!
//! `status` crosses as a **String** for the same reason `kind`/`fill` do on
//! `DocumentSchema` (Plan 19 Stage 7): a string lets the app send a bad value
//! and get the exact allowlist error back, where an enum would make an unknown
//! unrepresentable and the reject path untestable from Swift.

use murmur_core::{Job as CoreJob, JobStatus, NewJob};

use crate::engine::{EngineError, MurmurEngine};

/// The status values `set_job_status` accepts. Mirrors `JobStatus`.
pub const VALID_JOB_STATUSES: [&str; 3] = ["active", "done", "archived"];

/// FFI mirror of `murmur_core::Job` (murmur-core stays UniFFI-free — every
/// record lives on this side of the boundary, the `NotesEntry` precedent).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Job {
    pub id: String,
    /// What the operator calls it. The ONLY field the create UI collects —
    /// contractors name jobs however they think of them ("Alder Court",
    /// "the Hendersons", "412 Alder Ct"), and forcing that into separate
    /// client/site fields would impose a taxonomy they didn't ask for.
    pub name: String,
    /// Reserved: the model carries these and sync round-trips them, but the
    /// app doesn't collect them yet.
    pub client: Option<String>,
    pub site: Option<String>,
    /// Unix seconds; `None` = unscheduled/backlog.
    pub scheduled_at: Option<u64>,
    /// One of `VALID_JOB_STATUSES`.
    pub status: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

fn status_str(status: JobStatus) -> String {
    match status {
        JobStatus::Active => "active",
        JobStatus::Done => "done",
        JobStatus::Archived => "archived",
    }
    .to_string()
}

fn status_from_str(raw: &str) -> Option<JobStatus> {
    match raw {
        "active" => Some(JobStatus::Active),
        "done" => Some(JobStatus::Done),
        "archived" => Some(JobStatus::Archived),
        _ => None,
    }
}

fn job_from_core(job: CoreJob) -> Job {
    Job {
        id: job.id,
        name: job.name,
        client: job.client,
        site: job.site,
        scheduled_at: job.scheduled_at,
        status: status_str(job.status),
        created_at: job.created_at,
        updated_at: job.updated_at,
        device_id: job.device_id,
    }
}

#[uniffi::export]
impl MurmurEngine {
    /// Live jobs, newest first.
    pub fn list_jobs(&self) -> Result<Vec<Job>, EngineError> {
        let store = self.store.lock().map_err(|_| Self::job_err("store lock poisoned"))?;
        let jobs = store.list_jobs().map_err(|e| Self::job_err(e.to_string()))?;
        Ok(jobs.into_iter().map(job_from_core).collect())
    }

    /// Create a job from a name. Returns the created job so the board picks up
    /// the minted id and timestamps in one round-trip (the
    /// `save_document_schema` precedent).
    ///
    /// An empty or whitespace-only name is REJECTED rather than coerced to a
    /// placeholder (R6): a job called "Untitled" is worse than an error,
    /// because it looks deliberate and the operator has no idea which one it
    /// was meant to be.
    pub fn create_job(&self, name: String) -> Result<Job, EngineError> {
        if name.trim().is_empty() {
            return Err(Self::job_err("job name is empty"));
        }
        let store = self.store.lock().map_err(|_| Self::job_err("store lock poisoned"))?;
        let job = store
            .create_job(NewJob { name: name.trim().to_string(), ..Default::default() })
            .map_err(|e| Self::job_err(e.to_string()))?;
        Ok(job_from_core(job))
    }

    /// Move a job between active / done / archived. Returns the updated job.
    pub fn set_job_status(&self, id: String, status: String) -> Result<Job, EngineError> {
        let parsed = status_from_str(&status).ok_or_else(|| {
            Self::job_err(format!(
                "invalid job status '{status}'; must be one of: {}",
                VALID_JOB_STATUSES.join(", ")
            ))
        })?;
        let store = self.store.lock().map_err(|_| Self::job_err("store lock poisoned"))?;
        let job = store
            .update_job_status(&id, parsed)
            .map_err(|e| Self::job_err(e.to_string()))?;
        Ok(job_from_core(job))
    }
}

impl MurmurEngine {
    fn job_err(msg: impl Into<String>) -> EngineError {
        EngineError::Job(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_the_boundary() {
        for raw in VALID_JOB_STATUSES {
            let parsed = status_from_str(raw).expect("allowlisted status must parse");
            assert_eq!(status_str(parsed), raw);
        }
    }

    #[test]
    fn unknown_status_is_rejected_not_coerced() {
        // R6: an unknown value must surface as an error the app can show, not
        // silently become Active — which would flip a finished job back to
        // live work without anyone noticing.
        assert!(status_from_str("in_progress").is_none());
        assert!(status_from_str("").is_none());
        assert!(status_from_str("ACTIVE").is_none(), "matching is exact, not case-folded");
    }
}
