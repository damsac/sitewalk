use rusqlite::Row;

use crate::domain::{Job, JobStatus, NewJob};
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

fn job_from_row(row: &Row) -> Result<Job, CoreError> {
    let status_raw: String = row.get("status").map_err(CoreError::Sqlite)?;
    Ok(Job {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        name: row.get("name").map_err(CoreError::Sqlite)?,
        client: row.get("client").map_err(CoreError::Sqlite)?,
        site: row.get("site").map_err(CoreError::Sqlite)?,
        scheduled_at: row
            .get::<_, Option<i64>>("scheduled_at")
            .map_err(CoreError::Sqlite)?
            .map(|v| v as u64),
        status: JobStatus::parse(&status_raw)?,
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        updated_at: row.get::<_, i64>("updated_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
    })
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
            Some(row) => job_from_row(row),
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
            jobs.push(job_from_row(row)?);
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
