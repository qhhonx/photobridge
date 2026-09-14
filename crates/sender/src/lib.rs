//! Persistent sender decisions shared by Rust and native background executors.
//! References are opaque to this crate; hosts resolve them to exported resources.
use fs2::FileExt;
use photobridge_core::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::Path,
};

mod sorting;
pub use sorting::JobQuery;

fn db(e: rusqlite::Error) -> Error {
    Error::Storage(e.to_string())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Waiting,
    Paused,
    Failed,
    Received,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub receiver_id: String,
    pub asset: Asset,
    pub sources: BTreeMap<String, String>,
    pub state: JobState,
    pub generation: i64,
    pub attempts: u32,
    pub next_attempt_at: i64,
    pub confirmed_bytes: u64,
    pub error_code: Option<String>,
    pub native_task_id: Option<String>,
    #[serde(default)]
    pub state_changed_at_ms: Option<i64>,
    #[serde(default)]
    pub sort_value: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub job_id: i64,
    pub generation: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Network,
    Capacity,
    LowSpace,
    Busy,
    Authentication,
    Integrity,
    SourceUnavailable,
    Unsupported,
}
impl Failure {
    pub fn from_error(error: &Error) -> Self {
        match error {
            Error::Capacity => Self::Capacity,
            Error::LowSpace => Self::LowSpace,
            Error::Integrity => Self::Integrity,
            Error::Unsupported(_) | Error::Invalid(_) => Self::Unsupported,
            Error::Storage(_) | Error::NotFound => Self::SourceUnavailable,
            Error::Conflict(_) => Self::Busy,
            Error::Unauthorized => Self::Authentication,
            _ => Self::Network,
        }
    }
    fn automatic(&self) -> bool {
        matches!(
            self,
            Self::Network | Self::Capacity | Self::LowSpace | Self::Busy
        )
    }
}
impl Job {
    pub fn attempt(&self) -> Attempt {
        Attempt {
            job_id: self.id,
            generation: self.generation,
        }
    }
}

pub struct Sender {
    conn: Connection,
    _lock: File,
}
impl Sender {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.as_ref().join("sender.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| Error::Conflict("sender already open".into()))?;
        let conn = Connection::open(root.as_ref().join("sender.sqlite3")).map_err(db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS jobs (
                id INTEGER PRIMARY KEY, receiver_id TEXT NOT NULL, asset_id TEXT NOT NULL,
                manifest TEXT NOT NULL, sources TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'queued',
                generation INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0,
                next_attempt_at INTEGER NOT NULL DEFAULT 0, confirmed_bytes INTEGER NOT NULL DEFAULT 0,
                error_code TEXT, native_task_id TEXT,
                UNIQUE(receiver_id,asset_id));
            CREATE INDEX IF NOT EXISTS jobs_due ON jobs(receiver_id,state,next_attempt_at,id);
            CREATE TABLE IF NOT EXISTS native_checkpoints(job_id INTEGER PRIMARY KEY,status TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS receiver_features(receiver_id TEXT PRIMARY KEY,bundle_upload INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value INTEGER NOT NULL);
            INSERT OR IGNORE INTO settings VALUES('paused',0);
            INSERT OR IGNORE INTO settings VALUES('concurrent_uploads',4);
            INSERT OR IGNORE INTO settings VALUES('revision',0);
            CREATE TRIGGER IF NOT EXISTS jobs_insert_revision AFTER INSERT ON jobs BEGIN UPDATE settings SET value=value+1 WHERE key='revision'; END;
            CREATE TRIGGER IF NOT EXISTS jobs_update_revision AFTER UPDATE ON jobs BEGIN UPDATE settings SET value=value+1 WHERE key='revision'; END;
            CREATE INDEX IF NOT EXISTS jobs_source_revision ON jobs(receiver_id,json_extract(manifest,'$.source_id'),json_extract(manifest,'$.revision'));").map_err(db)?;
        sorting::migrate(&conn)?;
        // Rust foreground tasks cannot survive a process exit. OS tasks can and must
        // be reconciled explicitly after the native scheduler has enumerated them.
        conn.execute("UPDATE jobs SET state='queued', generation=generation+1 WHERE state='running' AND native_task_id IS NULL", []).map_err(db)?;
        conn.execute("DELETE FROM native_checkpoints WHERE job_id IN (SELECT id FROM jobs WHERE state!='running' AND state!='queued')", []).map_err(db)?;
        Ok(Self { conn, _lock: lock })
    }
    pub fn enqueue(
        &mut self,
        receiver: &str,
        asset: Asset,
        sources: BTreeMap<String, String>,
    ) -> Result<Job> {
        let id = asset.id()?;
        if receiver.is_empty()
            || receiver.len() > 256
            || receiver.chars().any(char::is_control)
            || sources.len() != asset.resources.len()
            || asset.resources.iter().any(|r| {
                sources
                    .get(&r.sha256)
                    .is_none_or(|s| s.is_empty() || s.len() > 8192)
            })
        {
            return Err(Error::Invalid("sender input".into()));
        }
        self.conn
            .execute(
                "INSERT INTO jobs(receiver_id,asset_id,manifest,sources) VALUES(?1,?2,?3,?4)
                ON CONFLICT(receiver_id,asset_id) DO UPDATE SET sources=excluded.sources,
                state='queued',error_code=NULL,next_attempt_at=0,generation=jobs.generation+1
                WHERE jobs.state='failed' AND jobs.error_code='source_unavailable' ",
                params![
                    receiver,
                    id,
                    serde_json::to_string(&asset)?,
                    serde_json::to_string(&sources)?
                ],
            )
            .map_err(db)?;
        let job_id = self
            .conn
            .query_row(
                "SELECT id FROM jobs WHERE receiver_id=?1 AND asset_id=?2",
                params![receiver, id],
                |r| r.get(0),
            )
            .map_err(db)?;
        self.job(job_id)
    }
    pub fn job(&self, id: i64) -> Result<Job> {
        let row = self.conn.query_row("SELECT receiver_id,manifest,sources,state,generation,attempts,next_attempt_at,confirmed_bytes,error_code,native_task_id FROM jobs WHERE id=?1", [id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,u32>(5)?,r.get::<_,i64>(6)?,r.get::<_,i64>(7)?,r.get::<_,Option<String>>(8)?,r.get::<_,Option<String>>(9)?))).optional().map_err(db)?.ok_or(Error::NotFound)?;
        Ok(Job {
            id,
            receiver_id: row.0,
            asset: serde_json::from_str(&row.1)?,
            sources: serde_json::from_str(&row.2)?,
            state: serde_json::from_value(serde_json::Value::String(row.3))?,
            generation: row.4,
            attempts: row.5,
            next_attempt_at: row.6,
            confirmed_bytes: row.7 as u64,
            error_code: row.8,
            native_task_id: row.9,
            state_changed_at_ms: self
                .conn
                .query_row(
                    "SELECT state_changed_at_ms FROM jobs WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(db)?,
            sort_value: None,
        })
    }
    /// Bounded keyset pagination; never materialize the entire library for a UI poll.
    pub fn list(&self, after: i64, limit: u32) -> Result<Vec<Job>> {
        self.list_filtered(after, limit, None, None)
    }
    pub fn list_filtered(
        &self,
        after: i64,
        limit: u32,
        receiver: Option<&str>,
        state: Option<&str>,
    ) -> Result<Vec<Job>> {
        self.list_filtered_ordered(after, limit, receiver, state, false)
    }
    /// Order the complete filtered result before keyset pagination.
    pub fn list_filtered_ordered(
        &self,
        after: i64,
        limit: u32,
        receiver: Option<&str>,
        state: Option<&str>,
        descending: bool,
    ) -> Result<Vec<Job>> {
        let sql = if descending {
            "SELECT id FROM jobs WHERE (?1=0 OR id<?1) AND (?3 IS NULL OR receiver_id=?3) AND (?4 IS NULL OR state=?4) ORDER BY id DESC LIMIT ?2"
        } else {
            "SELECT id FROM jobs WHERE id>?1 AND (?3 IS NULL OR receiver_id=?3) AND (?4 IS NULL OR state=?4) ORDER BY id ASC LIMIT ?2"
        };
        let mut s = self.conn.prepare(sql).map_err(db)?;
        let ids = s
            .query_map(params![after, limit.clamp(1, 500), receiver, state], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        ids.into_iter().map(|id| self.job(id)).collect()
    }
    pub fn source_in_use(&self, path: &str) -> Result<bool> {
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM jobs,json_each(jobs.sources) WHERE jobs.state!='received' AND json_each.value=?1)", [path], |r|r.get(0)).map_err(db)
    }
    /// Update fully host-verified locations without changing asset identity or receipts.
    /// A repaired source failure becomes eligible again; explicit pauses survive.
    pub fn rebind_sources(&mut self, id: i64, sources: &BTreeMap<String, String>) -> Result<()> {
        let job = self.job(id)?;
        if !sources.keys().eq(job.sources.keys()) {
            return Err(Error::Invalid("source mapping".into()));
        }
        let transaction = self.conn.transaction().map_err(db)?;
        transaction
            .execute(
                "UPDATE jobs SET sources=?1 WHERE id=?2",
                params![serde_json::to_string(sources)?, id],
            )
            .map_err(db)?;
        transaction.execute("UPDATE jobs SET state='queued',next_attempt_at=0,error_code=NULL,generation=generation+1 WHERE id=?1 AND state='failed' AND error_code='source_unavailable'", [id]).map_err(db)?;
        transaction.commit().map_err(db)?;
        Ok(())
    }
    pub fn revision(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key='revision'", [], |r| {
                r.get(0)
            })
            .map_err(db)
    }
    /// Bounded lookup for the visible library window, using an expression index.
    pub fn source_states(
        &self,
        receiver: &str,
        sources: &[(String, String)],
    ) -> Result<BTreeMap<String, String>> {
        if sources.len() > 400 {
            return Err(Error::Invalid("source window".into()));
        }
        let mut stmt = self.conn.prepare("SELECT state FROM jobs WHERE receiver_id=?1 AND json_extract(manifest,'$.source_id')=?2 AND json_extract(manifest,'$.revision')=?3 ORDER BY id DESC LIMIT 1").map_err(db)?;
        let mut states = BTreeMap::new();
        for (id, revision) in sources {
            let value: Option<String> = stmt
                .query_row(params![receiver, id, revision], |r| r.get(0))
                .optional()
                .map_err(db)?;
            if let Some(value) = value {
                states.insert(id.clone(), value);
            }
        }
        Ok(states)
    }
    pub fn summary(&self, receiver: &str) -> Result<serde_json::Value> {
        let mut stmt = self.conn.prepare("SELECT state,COUNT(*),SUM(confirmed_bytes) FROM jobs WHERE receiver_id=?1 GROUP BY state").map_err(db)?;
        let rows = stmt
            .query_map([receiver], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(db)?;
        let mut result = serde_json::json!({"total":0,"confirmed_bytes":0,"received":0,"waiting":0,"failed":0,"queued":0,"running":0,"paused":0});
        let mut count = 0;
        let mut bytes = 0;
        for row in rows {
            let (state, n, b) = row.map_err(db)?;
            result[&state] = n.into();
            count += n;
            bytes += b;
        }
        result["total"] = count.into();
        result["confirmed_bytes"] = bytes.into();
        let waiting: Option<(String,i64)> = self.conn.query_row("SELECT error_code,next_attempt_at FROM jobs WHERE receiver_id=?1 AND state='waiting' ORDER BY next_attempt_at,id LIMIT 1",[receiver],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db)?;
        result["waiting_reason"] = waiting
            .as_ref()
            .map(|r| serde_json::json!(r.0))
            .unwrap_or(serde_json::Value::Null);
        result["next_retry_at"] = waiting
            .map(|r| serde_json::json!(r.1))
            .unwrap_or(serde_json::Value::Null);
        Ok(result)
    }
    pub fn paused(&self) -> Result<bool> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key='paused'", [], |r| {
                r.get(0)
            })
            .map_err(db)
    }
    /// Global pause retains attempt bindings so late completion can be reconciled.
    /// The executor must cancel/suspend active network operations when this returns.
    pub fn set_paused(&mut self, paused: bool) -> Result<()> {
        self.conn
            .execute("UPDATE settings SET value=?1 WHERE key='paused'", [paused])
            .map_err(db)?;
        Ok(())
    }
    pub fn claim(&mut self, receiver: &str, now: i64) -> Result<Option<Job>> {
        if now < 0 {
            return Err(Error::Invalid("time".into()));
        }
        if self.paused()? {
            return Ok(None);
        }
        let tx = self.conn.transaction().map_err(db)?;
        let id: Option<i64> = tx.query_row("SELECT id FROM jobs WHERE receiver_id=?1 AND (state='queued' OR (state='waiting' AND next_attempt_at<=?2)) ORDER BY CAST(json_extract(manifest,'$.metadata.created_at_ms') AS INTEGER) DESC,json_extract(manifest,'$.source_id'),id LIMIT 1",params![receiver,now],|r|r.get(0)).optional().map_err(db)?;
        if let Some(id) = id {
            tx.execute("UPDATE jobs SET state='running',generation=generation+1,attempts=attempts+1,error_code=NULL,native_task_id=NULL WHERE id=?1",[id]).map_err(db)?;
        }
        tx.commit().map_err(db)?;
        id.map(|id| self.job(id)).transpose()
    }
    fn current(&self, a: &Attempt) -> Result<Job> {
        let job = self.job(a.job_id)?;
        if job.generation != a.generation || job.state != JobState::Running {
            return Err(Error::Conflict("superseded attempt".into()));
        }
        Ok(job)
    }
    pub fn bind_native_task(&mut self, a: &Attempt, task_id: &str) -> Result<()> {
        let job = self.current(a)?;
        if self.paused()?
            || job
                .native_task_id
                .as_deref()
                .is_some_and(|id| id != task_id)
        {
            return Err(Error::Conflict("native task binding".into()));
        }
        if task_id.is_empty() || task_id.len() > 512 {
            return Err(Error::Invalid("native task id".into()));
        }
        self.conn
            .execute(
                "UPDATE jobs SET native_task_id=?1 WHERE id=?2",
                params![task_id, a.job_id],
            )
            .map_err(db)?;
        Ok(())
    }
    /// Features are accepted only after an authenticated capability check.
    pub fn set_bundle_upload(&mut self, receiver: &str, enabled: bool) -> Result<()> {
        self.conn.execute("INSERT INTO receiver_features VALUES(?1,?2) ON CONFLICT(receiver_id) DO UPDATE SET bundle_upload=excluded.bundle_upload", params![receiver, enabled]).map_err(db)?;
        Ok(())
    }
    pub fn bundle_upload(&self, receiver: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT bundle_upload FROM receiver_features WHERE receiver_id=?1",
                [receiver],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?
            .unwrap_or(false))
    }
    /// Queue a bounded window with the OS so later uploads do not need a fresh
    /// application wake-up. Legacy multi-request transfers remain serial.
    pub fn concurrent_uploads(&self) -> Result<u32> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key='concurrent_uploads'",
                [],
                |r| r.get(0),
            )
            .map_err(db)
    }
    pub fn set_concurrent_uploads(&mut self, limit: u32) -> Result<()> {
        if !(1..=4).contains(&limit) {
            return Err(Error::Invalid("concurrent uploads".into()));
        }
        self.conn
            .execute(
                "UPDATE settings SET value=?1 WHERE key='concurrent_uploads'",
                [limit],
            )
            .map_err(db)?;
        Ok(())
    }
    pub fn claim_native(&mut self, receiver: &str, now: i64) -> Result<Option<Job>> {
        let active: u32 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE receiver_id=?1 AND state='running'",
                [receiver],
                |r| r.get(0),
            )
            .map_err(db)?;
        let window = if self.bundle_upload(receiver)? {
            self.concurrent_uploads()?
        } else {
            1
        };
        if active >= window {
            return Ok(None);
        }
        self.claim(receiver, now)
    }
    pub fn checkpoint(&self, id: i64) -> Result<Option<AssetStatus>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM native_checkpoints WHERE job_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
    pub fn check_native(&self, a: &Attempt, task_id: &str) -> Result<()> {
        if self.current(a)?.native_task_id.as_deref() != Some(task_id) {
            return Err(Error::Conflict("superseded native task".into()));
        }
        Ok(())
    }
    /// Persist acknowledgement and release the OS binding in the same transaction.
    pub fn complete_native(
        &mut self,
        a: &Attempt,
        task_id: &str,
        status: &AssetStatus,
    ) -> Result<()> {
        self.check_native(a, task_id)?;
        let job = self.current(a)?;
        next_action(&job.asset, status, MAX_CHUNK_BYTES)?;
        let bytes = status
            .resources
            .iter()
            .try_fold(0u64, |n, r| n.checked_add(r.offset))
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or(Error::Capacity)?;
        let tx = self.conn.transaction().map_err(db)?;
        tx.execute(
            "INSERT OR REPLACE INTO native_checkpoints VALUES(?1,?2)",
            params![a.job_id, serde_json::to_string(status)?],
        )
        .map_err(db)?;
        tx.execute("UPDATE jobs SET confirmed_bytes=?1,state=?2,native_task_id=NULL,attempts=0,next_attempt_at=0,error_code=NULL WHERE id=?3",
            params![bytes as i64, if status.receipt==ReceiptState::Received {"received"} else {"queued"},a.job_id]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(())
    }
    pub fn clear_checkpoint(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM native_checkpoints WHERE job_id=?1", [id])
            .map_err(db)?;
        Ok(())
    }
    /// Only receiver acknowledgements advance confirmed progress or mark receipt.
    pub fn acknowledge(&mut self, a: &Attempt, status: &AssetStatus) -> Result<()> {
        let job = self.current(a)?;
        next_action(&job.asset, status, MAX_CHUNK_BYTES)?;
        let bytes = status
            .resources
            .iter()
            .try_fold(0u64, |n, r| n.checked_add(r.offset))
            .ok_or(Error::Capacity)?;
        if bytes > i64::MAX as u64 {
            return Err(Error::Capacity);
        }
        self.conn.execute("UPDATE jobs SET confirmed_bytes=?1,state=?2,native_task_id=CASE WHEN ?2='received' THEN NULL ELSE native_task_id END WHERE id=?3",params![bytes as i64,if status.receipt==ReceiptState::Received {"received"} else {"running"},a.job_id]).map_err(db)?;
        Ok(())
    }
    pub fn fail(&mut self, a: &Attempt, failure: Failure, now: i64) -> Result<()> {
        let job = self.current(a)?;
        self.clear_checkpoint(a.job_id)?;
        // Stable per-job jitter, capped exponential backoff; no sleeping in core.
        let base = if matches!(failure, Failure::Capacity | Failure::LowSpace) {
            60i64
        } else {
            5
        };
        let delay = (base * (1i64 << job.attempts.saturating_sub(1).min(8))).min(900)
            + job.id.rem_euclid(7);
        let due = now.max(0).saturating_add(delay);
        let code = serde_json::to_value(&failure)?.as_str().unwrap().to_owned();
        self.conn.execute("UPDATE jobs SET state=?1,next_attempt_at=?2,error_code=?3,native_task_id=NULL WHERE id=?4",params![if failure.automatic(){"waiting"}else{"failed"},due,code,a.job_id]).map_err(db)?;
        Ok(())
    }
    /// Pause/cancel completion is not a failure and never resets received assets.
    pub fn interrupted(&mut self, a: &Attempt) -> Result<()> {
        self.current(a)?;
        self.clear_checkpoint(a.job_id)?;
        self.conn.execute("UPDATE jobs SET state='queued',generation=generation+1,native_task_id=NULL WHERE id=?1",[a.job_id]).map_err(db)?;
        Ok(())
    }
    pub fn pause_job(&mut self, id: i64) -> Result<Option<String>> {
        let job = self.job(id)?;
        if job.state == JobState::Received {
            return Ok(None);
        }
        self.conn.execute("UPDATE jobs SET state='paused',generation=generation+1,native_task_id=NULL WHERE id=?1",[id]).map_err(db)?;
        Ok(job.native_task_id)
    }
    pub fn retry(&mut self, id: i64) -> Result<()> {
        let job = self.job(id)?;
        if matches!(
            job.state,
            JobState::Waiting | JobState::Failed | JobState::Paused
        ) {
            self.conn.execute("UPDATE jobs SET state='queued',next_attempt_at=0,error_code=NULL,generation=generation+1 WHERE id=?1",[id]).map_err(db)?;
        }
        Ok(())
    }
    /// Called after authenticating the existing peer at its recovered route.
    /// Do not resume global/per-job pauses or retry unrelated content failures.
    pub fn recover_connection(&mut self, receiver: &str) -> Result<usize> {
        self.conn.execute("UPDATE jobs SET state='queued',next_attempt_at=0,error_code=NULL,generation=generation+1 WHERE receiver_id=?1 AND state IN ('waiting','failed') AND error_code IN ('network','authentication')", [receiver]).map_err(db)
    }
    /// The host enumerates all live OS tasks in its session first. Returned IDs
    /// are stale tasks to cancel. Missing tasks are requeued and query receiver
    /// status before uploading, including when a final acknowledgement was lost.
    pub fn reconcile_native(&mut self, live: &BTreeSet<String>) -> Result<Vec<String>> {
        let mut stmt=self.conn.prepare("SELECT id,native_task_id FROM jobs WHERE state='running' AND native_task_id IS NOT NULL").map_err(db)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        drop(stmt);
        let known: BTreeSet<_> = rows.iter().map(|(_, t)| t.clone()).collect();
        let tx = self.conn.transaction().map_err(db)?;
        for (id, task) in rows {
            if !live.contains(&task) {
                tx.execute("DELETE FROM native_checkpoints WHERE job_id=?1", [id])
                    .map_err(db)?;
                tx.execute("UPDATE jobs SET state='queued',generation=generation+1,native_task_id=NULL WHERE id=?1",[id]).map_err(db)?;
            }
        }
        tx.commit().map_err(db)?;
        Ok(live.difference(&known).cloned().collect())
    }
}
