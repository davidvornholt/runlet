use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Held,
    CleanupPending,
    Cleaned,
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Held => "held",
            Self::CleanupPending => "cleanup_pending",
            Self::Cleaned => "cleaned",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "held" => Self::Held,
            "cleanup_pending" => Self::CleanupPending,
            "cleaned" => Self::Cleaned,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: String,
    pub github_job_id: i64,
    pub repository: String,
    pub runner_name: String,
    pub container_name: String,
    pub workspace: String,
    pub status: JobStatus,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> rusqlite::Result<Self> {
        let connection = Connection::open_in_memory()?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn migrate(&self) -> rusqlite::Result<()> {
        self.connection.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS runner_registrations (
                runner_name TEXT PRIMARY KEY,
                repository TEXT NOT NULL,
                github_runner_id INTEGER,
                token_expires_at TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                revoked_at TEXT
            );

            CREATE TABLE IF NOT EXISTS jobs (
                job_id TEXT PRIMARY KEY,
                github_job_id INTEGER NOT NULL DEFAULT 0,
                repository TEXT NOT NULL,
                runner_name TEXT NOT NULL,
                container_name TEXT NOT NULL,
                workspace TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                cleaned_at TEXT
            );

            CREATE UNIQUE INDEX IF NOT EXISTS jobs_repository_github_job_id_idx
            ON jobs(repository, github_job_id)
            WHERE github_job_id != 0;

            CREATE TABLE IF NOT EXISTS lifecycle_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS cache_entries (
                namespace TEXT NOT NULL,
                key TEXT NOT NULL,
                repository TEXT NOT NULL,
                trusted_writer INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (namespace, key)
            );
            "#,
        )?;
        add_column_if_missing(
            &self.connection,
            "jobs",
            "github_job_id",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        Ok(())
    }

    pub fn upsert_job(&self, record: &JobRecord) -> rusqlite::Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO jobs (
                job_id, github_job_id, repository, runner_name, container_name, workspace, status
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(job_id) DO UPDATE SET
                github_job_id = excluded.github_job_id,
                repository = excluded.repository,
                runner_name = excluded.runner_name,
                container_name = excluded.container_name,
                workspace = excluded.workspace,
                status = excluded.status,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                record.job_id,
                record.github_job_id,
                record.repository,
                record.runner_name,
                record.container_name,
                record.workspace,
                record.status.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn job(&self, job_id: &str) -> rusqlite::Result<Option<JobRecord>> {
        self.connection
            .query_row(
                r#"
                SELECT job_id, github_job_id, repository, runner_name, container_name, workspace, status
                FROM jobs
                WHERE job_id = ?1
                "#,
                [job_id],
                |row| {
                    Ok(JobRecord {
                        job_id: row.get(0)?,
                        github_job_id: row.get(1)?,
                        repository: row.get(2)?,
                        runner_name: row.get(3)?,
                        container_name: row.get(4)?,
                        workspace: row.get(5)?,
                        status: JobStatus::from_str(row.get::<_, String>(6)?.as_str()),
                    })
                },
            )
            .optional()
    }

    pub fn job_by_github_job_id(
        &self,
        repository: &str,
        github_job_id: i64,
    ) -> rusqlite::Result<Option<JobRecord>> {
        self.connection
            .query_row(
                r#"
                SELECT job_id, github_job_id, repository, runner_name, container_name, workspace, status
                FROM jobs
                WHERE repository = ?1 AND github_job_id = ?2
                "#,
                params![repository, github_job_id],
                |row| {
                    Ok(JobRecord {
                        job_id: row.get(0)?,
                        github_job_id: row.get(1)?,
                        repository: row.get(2)?,
                        runner_name: row.get(3)?,
                        container_name: row.get(4)?,
                        workspace: row.get(5)?,
                        status: JobStatus::from_str(row.get::<_, String>(6)?.as_str()),
                    })
                },
            )
            .optional()
    }

    pub fn delete_job(&self, job_id: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "DELETE FROM lifecycle_events WHERE job_id = ?1",
            params![job_id],
        )?;
        self.connection
            .execute("DELETE FROM jobs WHERE job_id = ?1", params![job_id])?;
        Ok(())
    }

    pub fn set_job_status(&self, job_id: &str, status: JobStatus) -> rusqlite::Result<()> {
        self.connection.execute(
            r#"
            UPDATE jobs
            SET status = ?2,
                updated_at = CURRENT_TIMESTAMP,
                cleaned_at = CASE WHEN ?2 = 'cleaned' THEN CURRENT_TIMESTAMP ELSE cleaned_at END
            WHERE job_id = ?1
            "#,
            params![job_id, status.as_str()],
        )?;
        Ok(())
    }

    pub fn append_event(
        &self,
        job_id: &str,
        event_type: &str,
        message: &str,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO lifecycle_events (job_id, event_type, message)
            VALUES (?1, ?2, ?3)
            "#,
            params![job_id, event_type, message],
        )?;
        Ok(())
    }

    pub fn mark_interrupted_jobs_cleanup_pending(&self) -> rusqlite::Result<()> {
        self.connection.execute(
            r#"
            UPDATE jobs
            SET status = 'cleanup_pending',
                updated_at = CURRENT_TIMESTAMP
            WHERE status IN ('queued', 'running', 'succeeded', 'failed')
            "#,
            [],
        )?;
        Ok(())
    }

    pub fn cleanup_pending_jobs(&self) -> rusqlite::Result<Vec<JobRecord>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT job_id, github_job_id, repository, runner_name, container_name, workspace, status
            FROM jobs
            WHERE status = 'cleanup_pending'
            ORDER BY created_at ASC
            "#,
        )?;
        let records = statement
            .query_map([], |row| {
                Ok(JobRecord {
                    job_id: row.get(0)?,
                    github_job_id: row.get(1)?,
                    repository: row.get(2)?,
                    runner_name: row.get(3)?,
                    container_name: row.get(4)?,
                    workspace: row.get(5)?,
                    status: JobStatus::from_str(row.get::<_, String>(6)?.as_str()),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    pub fn record_runner_registration(
        &self,
        runner_name: &str,
        repository: &str,
        token_expires_at: &str,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO runner_registrations (
                runner_name, repository, token_expires_at
            )
            VALUES (?1, ?2, ?3)
            ON CONFLICT(runner_name) DO UPDATE SET
                repository = excluded.repository,
                token_expires_at = excluded.token_expires_at,
                revoked_at = NULL
            "#,
            params![runner_name, repository, token_expires_at],
        )?;
        Ok(())
    }

    pub fn mark_runner_revoked(&self, runner_name: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            r#"
            UPDATE runner_registrations
            SET revoked_at = CURRENT_TIMESTAMP
            WHERE runner_name = ?1
            "#,
            [runner_name],
        )?;
        Ok(())
    }

    pub fn upsert_cache_entry(
        &self,
        namespace: &str,
        key: &str,
        repository: &str,
        trusted_writer: bool,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            r#"
            INSERT INTO cache_entries (namespace, key, repository, trusted_writer)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(namespace, key) DO UPDATE SET
                repository = excluded.repository,
                trusted_writer = excluded.trusted_writer,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![namespace, key, repository, trusted_writer],
        )?;
        Ok(())
    }
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|existing| existing == column);
    if !exists {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(job_id: &str, status: JobStatus) -> JobRecord {
        JobRecord {
            job_id: job_id.to_string(),
            github_job_id: job_id.parse().unwrap_or_default(),
            repository: "github:org/project".to_string(),
            runner_name: format!("runner-{job_id}"),
            container_name: format!("runlet-{job_id}"),
            workspace: format!("/var/lib/runlet/jobs/{job_id}"),
            status,
        }
    }

    #[test]
    fn stores_job_and_updates_status() {
        let store = Store::open_memory().expect("store should open");
        store
            .upsert_job(&record("1", JobStatus::Queued))
            .expect("job should insert");
        store
            .set_job_status("1", JobStatus::Running)
            .expect("status should update");

        let job = store.job("1").expect("job query should work").unwrap();
        assert_eq!(job.status, JobStatus::Running);
    }

    #[test]
    fn finds_job_by_repository_scoped_github_job_id() {
        let store = Store::open_memory().expect("store should open");
        store
            .upsert_job(&record("42", JobStatus::Queued))
            .expect("job should insert");

        let job = store
            .job_by_github_job_id("github:org/project", 42)
            .expect("job lookup should work")
            .expect("job should exist");
        assert_eq!(job.job_id, "42");

        assert!(store
            .upsert_job(&JobRecord {
                job_id: "other".to_string(),
                github_job_id: 42,
                repository: "github:org/project".to_string(),
                runner_name: "runner-other".to_string(),
                container_name: "runlet-other".to_string(),
                workspace: "/var/lib/runlet/jobs/other".to_string(),
                status: JobStatus::Queued,
            })
            .is_err());
    }

    #[test]
    fn finds_explicit_cleanup_pending_jobs() {
        let store = Store::open_memory().expect("store should open");
        store
            .upsert_job(&record("1", JobStatus::Running))
            .expect("job should insert");
        store
            .upsert_job(&record("2", JobStatus::Succeeded))
            .expect("job should insert");
        store
            .upsert_job(&record("3", JobStatus::CleanupPending))
            .expect("job should insert");
        store
            .upsert_job(&record("4", JobStatus::Failed))
            .expect("job should insert");

        let jobs = store.cleanup_pending_jobs().expect("query should work");
        let job_ids = jobs.into_iter().map(|job| job.job_id).collect::<Vec<_>>();
        assert_eq!(job_ids, ["3"]);
    }

    #[test]
    fn marks_interrupted_allocated_jobs_for_cleanup_on_startup() {
        let store = Store::open_memory().expect("store should open");
        store
            .upsert_job(&record("0", JobStatus::Queued))
            .expect("job should insert");
        store
            .upsert_job(&record("1", JobStatus::Running))
            .expect("job should insert");
        store
            .upsert_job(&record("2", JobStatus::Succeeded))
            .expect("job should insert");
        store
            .upsert_job(&record("3", JobStatus::Failed))
            .expect("job should insert");
        store
            .upsert_job(&record("4", JobStatus::Held))
            .expect("job should insert");

        store
            .mark_interrupted_jobs_cleanup_pending()
            .expect("interrupted jobs should update");

        let queued_job = store.job("0").expect("job query should work").unwrap();
        assert_eq!(queued_job.status, JobStatus::CleanupPending);
        let running_job = store.job("1").expect("job query should work").unwrap();
        assert_eq!(running_job.status, JobStatus::CleanupPending);
        let succeeded_job = store.job("2").expect("job query should work").unwrap();
        assert_eq!(succeeded_job.status, JobStatus::CleanupPending);
        let failed_job = store.job("3").expect("job query should work").unwrap();
        assert_eq!(failed_job.status, JobStatus::CleanupPending);
        let held_job = store.job("4").expect("job query should work").unwrap();
        assert_eq!(held_job.status, JobStatus::Held);
    }
}
