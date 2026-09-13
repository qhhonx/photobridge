//! Private runtime settings, bounded diagnostics, and managed sender-cache policy.
mod history;
use super::*;
pub use history::HistoryAction;
use rusqlite::{params, Connection};

#[cfg(test)]
mod ordering_tests {
    use super::*;

    #[test]
    fn legacy_pending_queue_is_backfilled_without_losing_retries_or_membership() {
        let root = std::env::temp_dir().join(format!("photobridge-order-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let conn = Connection::open(root.join("maintenance.sqlite3")).unwrap();
        conn.execute_batch("CREATE TABLE pending_sources(receiver TEXT NOT NULL,source TEXT NOT NULL,retry_at INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(receiver,source));
            INSERT INTO pending_sources VALUES('a','old',0),('a','new',0),('a','retry',9223372036854775807),('b','other',0);").unwrap();
        drop(conn);
        let m = Maintenance::open(&root).unwrap();
        assert_eq!(
            m.pending("a").unwrap()["unordered"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        m.source_dates(
            "a",
            &[
                ("old".into(), 100),
                ("new".into(), 900),
                ("retry".into(), 1000),
            ],
        )
        .unwrap();
        drop(m);
        let m = Maintenance::open(&root).unwrap();
        assert_eq!(m.pending("a").unwrap()["sources"], json!(["new", "old"]));
        assert_eq!(m.pending("a").unwrap()["count"], 3);
        assert_eq!(m.pending("a").unwrap()["unordered"], json!([]));
        assert_eq!(m.pending("b").unwrap()["unordered"], json!(["other"]));
        m.conn
            .execute(
                "UPDATE pending_sources SET retry_at=0 WHERE receiver='a' AND source='retry'",
                [],
            )
            .unwrap();
        assert_eq!(
            m.pending("a").unwrap()["sources"],
            json!(["retry", "new", "old"])
        );
        drop(m);
        fs::remove_dir_all(root).unwrap();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub cache_budget_bytes: u64,
    pub receiver_budget_bytes: u64,
    pub min_free_bytes: u64,
    pub auto_reclaim: bool,
    #[serde(default)]
    pub receiver_relay: bool,
    pub log_days: u32,
    pub log_limit: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            cache_budget_bytes: 5 << 30,
            receiver_budget_bytes: 6 << 30,
            min_free_bytes: 1 << 30,
            auto_reclaim: true,
            receiver_relay: false,
            log_days: 14,
            log_limit: 5000,
        }
    }
}
pub struct Maintenance {
    conn: Connection,
    pub settings: Settings,
}
/// Diagnostics accept only counters and fixed vocabulary, never arbitrary text
/// that could accidentally export filenames, endpoints or credentials.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EventContext {
    pub queued: Option<u64>,
    pub running: Option<u64>,
    pub waiting: Option<u64>,
    pub failed: Option<u64>,
    pub pending_imports: Option<u64>,
    pub discovery_pending: Option<u64>,
    pub paused: Option<bool>,
    pub active_requests: Option<u32>,
    pub next_retry_at: Option<i64>,
    pub http_status: Option<u16>,
    pub system_error: Option<i64>,
    pub bytes_sent: Option<u64>,
    pub bytes_expected: Option<u64>,
    pub execution: Option<Execution>,
    pub phase: Option<RequestPhase>,
    pub reason: Option<DispatchReason>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Execution {
    Foreground,
    Background,
    Inactive,
    Desktop,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPhase {
    Bundle,
    Manifest,
    Upload,
    Commit,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchReason {
    Paused,
    Unpaired,
    ActiveRequest,
    NoEligibleJob,
    PreparationFailed,
}
fn database(e: rusqlite::Error) -> Error {
    Error::Storage(e.to_string())
}
impl Maintenance {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        let conn = Connection::open(root.join("maintenance.sqlite3")).map_err(database)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(database)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS configuration(id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS history_runs(receiver TEXT PRIMARY KEY, run INTEGER NOT NULL, state TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS history_members(receiver TEXT NOT NULL, source TEXT NOT NULL, revision TEXT NOT NULL, PRIMARY KEY(receiver,source));
          CREATE TABLE IF NOT EXISTS pending_sources(receiver TEXT NOT NULL, source TEXT NOT NULL, retry_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(receiver,source));
          CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, code TEXT NOT NULL, job_id INTEGER, amount INTEGER);") .map_err(database)?;
        let has_capture_date: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('pending_sources') WHERE name='created_at_ms')",
            [], |r| r.get(0)).map_err(database)?;
        if !has_capture_date {
            conn.execute(
                "ALTER TABLE pending_sources ADD COLUMN created_at_ms INTEGER",
                [],
            )
            .map_err(database)?;
        }
        conn.execute("CREATE INDEX IF NOT EXISTS pending_capture_order ON pending_sources(receiver,created_at_ms DESC,source)", []).map_err(database)?;
        let has_context: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('events') WHERE name='context')",
                [],
                |r| r.get(0),
            )
            .map_err(database)?;
        if !has_context {
            conn.execute("ALTER TABLE events ADD COLUMN context TEXT", [])
                .map_err(database)?;
        }
        let saved: Option<String> = conn
            .query_row("SELECT value FROM configuration WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(database)?;
        let settings = saved
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        let value = Self { conn, settings };
        value.prune()?;
        Ok(value)
    }
    pub fn save(&mut self, settings: Settings) -> Result<()> {
        if !(1..=(1u64 << 40)).contains(&settings.cache_budget_bytes)
            || !(1..=(1u64 << 40)).contains(&settings.receiver_budget_bytes)
            || !(0..=(100u64 << 30)).contains(&settings.min_free_bytes)
            || !(1..=90).contains(&settings.log_days)
            || !(100..=20000).contains(&settings.log_limit)
        {
            return Err(Error::Invalid("storage settings".into()));
        }
        self.conn.execute("INSERT INTO configuration VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value", [serde_json::to_string(&settings)?]).map_err(database)?;
        self.settings = settings;
        self.conn
            .execute("UPDATE pending_sources SET retry_at=0", [])
            .map_err(database)?;
        self.log("settings_changed", None, None)?;
        self.prune()
    }
    pub fn log(&self, code: &str, job: Option<i64>, amount: Option<u64>) -> Result<()> {
        self.log_context(code, job, amount, None)
    }
    pub fn log_context(
        &self,
        code: &str,
        job: Option<i64>,
        amount: Option<u64>,
        context: Option<&EventContext>,
    ) -> Result<()> {
        if code.is_empty()
            || code.len() > 48
            || !code.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
        {
            return Err(Error::Invalid("event code".into()));
        }
        self.conn
            .execute(
                "INSERT INTO events(ts,code,job_id,amount,context) VALUES(?1,?2,?3,?4,?5)",
                params![
                    now(),
                    code,
                    job,
                    amount.map(|v| v.min(i64::MAX as u64) as i64),
                    context.map(serde_json::to_string).transpose()?
                ],
            )
            .map_err(database)?;
        self.prune()
    }
    fn prune(&self) -> Result<()> {
        self.conn.execute("DELETE FROM events WHERE ts<?1 OR id NOT IN (SELECT id FROM events ORDER BY id DESC LIMIT ?2)",params![now()-i64::from(self.settings.log_days)*86400,self.settings.log_limit]).map_err(database)?;
        Ok(())
    }
    pub fn schedule_sources(&self, receiver: &str, sources: &[String]) -> Result<()> {
        if receiver.is_empty()
            || receiver.len() > 256
            || sources.len() > 10000
            || sources.iter().any(|s| s.is_empty() || s.len() > 8192)
        {
            return Err(Error::Invalid("pending sources".into()));
        }
        let tx = self.conn.unchecked_transaction().map_err(database)?;
        for source in sources {
            tx.execute(
                "INSERT OR IGNORE INTO pending_sources(receiver,source) VALUES(?1,?2)",
                params![receiver, source],
            )
            .map_err(database)?;
        }
        tx.commit().map_err(database)?;
        Ok(())
    }
    pub fn pending_states(
        &self,
        receiver: &str,
        sources: &[(String, String)],
    ) -> Result<std::collections::BTreeMap<String, String>> {
        if sources.len() > 400 {
            return Err(Error::Invalid("source window".into()));
        }
        let mut query = self
            .conn
            .prepare("SELECT COUNT(*) FROM pending_sources WHERE receiver=?1 AND source=?2")
            .map_err(database)?;
        let mut result = std::collections::BTreeMap::new();
        for (id, _) in sources {
            let count: i64 = query
                .query_row(params![receiver, id], |r| r.get(0))
                .map_err(database)?;
            if count > 0 {
                result.insert(id.clone(), "preparing".into());
            }
        }
        Ok(result)
    }
    /// Read-only, receiver-scoped browsing includes delayed preparation retries.
    pub fn source_page(&self, receiver: &str, history: bool, after: i64) -> Result<Value> {
        let tx = self.conn.unchecked_transaction().map_err(database)?;
        let (count_sql, page_sql) = if history {
            ("SELECT COUNT(*) FROM history_members WHERE receiver=?1",
             "SELECT h.rowid,h.source,h.revision,p.retry_at FROM history_members h LEFT JOIN pending_sources p ON p.receiver=h.receiver AND p.source=h.source WHERE h.receiver=?1 AND h.rowid>?2 ORDER BY h.rowid LIMIT 100")
        } else {
            ("SELECT COUNT(*) FROM pending_sources WHERE receiver=?1",
             "SELECT rowid,source,'',retry_at FROM pending_sources WHERE receiver=?1 AND rowid>?2 ORDER BY rowid LIMIT 100")
        };
        let total: i64 = tx
            .query_row(count_sql, [receiver], |r| r.get(0))
            .map_err(database)?;
        let run: Option<i64> = tx
            .query_row(
                "SELECT run FROM history_runs WHERE receiver=?1",
                [receiver],
                |r| r.get(0),
            )
            .optional()
            .map_err(database)?;
        let rows =
            {
                let mut query = tx.prepare(page_sql).map_err(database)?;
                let rows = query.query_map(params![receiver, after], |r| Ok(json!({
                "cursor": r.get::<_,i64>(0)?, "source": r.get::<_,String>(1)?,
                "revision": r.get::<_,String>(2)?, "retry_at": r.get::<_,Option<i64>>(3)?
            }))).map_err(database)?.collect::<std::result::Result<Vec<_>,_>>().map_err(database)?;
                rows
            };
        tx.commit().map_err(database)?;
        Ok(json!({"total":total,"run":run,"items":rows}))
    }
    pub fn pending(&self, receiver: &str) -> Result<Value> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pending_sources WHERE receiver=?1",
                [receiver],
                |r| r.get(0),
            )
            .map_err(database)?;
        let mut s=self.conn.prepare("SELECT source FROM pending_sources WHERE receiver=?1 AND retry_at<=?2 ORDER BY created_at_ms DESC,source LIMIT 5").map_err(database)?;
        let ids = s
            .query_map(params![receiver, now()], |r| r.get::<_, String>(0))
            .map_err(database)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(database)?;
        let mut query = self.conn.prepare("SELECT source FROM pending_sources WHERE receiver=?1 AND created_at_ms IS NULL ORDER BY rowid LIMIT 200").map_err(database)?;
        let unordered = query
            .query_map([receiver], |r| r.get::<_, String>(0))
            .map_err(database)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(database)?;
        Ok(json!({"count":count,"sources":ids,"unordered":unordered}))
    }
    /// Metadata-only backfill also orders queues created by older app versions.
    /// Missing or inaccessible dates sort last; no source membership is removed.
    pub fn source_dates(&self, receiver: &str, dates: &[(String, i64)]) -> Result<()> {
        if dates.len() > 200 {
            return Err(Error::Invalid("source dates".into()));
        }
        let tx = self.conn.unchecked_transaction().map_err(database)?;
        for (source, date) in dates {
            tx.execute(
                "UPDATE pending_sources SET created_at_ms=?3 WHERE receiver=?1 AND source=?2",
                params![receiver, source, date],
            )
            .map_err(database)?;
        }
        tx.commit().map_err(database)?;
        Ok(())
    }
    pub fn source_result(&self, receiver: &str, source: &str, complete: bool) -> Result<()> {
        if complete {
            self.conn
                .execute(
                    "DELETE FROM pending_sources WHERE receiver=?1 AND source=?2",
                    params![receiver, source],
                )
                .map_err(database)?;
        } else {
            self.conn
                .execute(
                    "UPDATE pending_sources SET retry_at=?3 WHERE receiver=?1 AND source=?2",
                    params![receiver, source, now() + 300],
                )
                .map_err(database)?;
        }
        Ok(())
    }
    pub fn events(&self) -> Result<Value> {
        self.prune()?;
        let mut s = self
            .conn
            .prepare("SELECT ts,code,job_id,amount,context FROM events ORDER BY id DESC LIMIT ?1")
            .map_err(database)?;
        let rows = s.query_map([self.settings.log_limit], |r| {
            let context = r.get::<_,Option<String>>(4)?;
            let context = context.map(|s| serde_json::from_str::<EventContext>(&s)).transpose()
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e)))?;
            Ok(json!({"timestamp":r.get::<_,i64>(0)?,"code":r.get::<_,String>(1)?,"job_id":r.get::<_,Option<i64>>(2)?,"bytes":r.get::<_,Option<i64>>(3)?,"context":context}))
        }).map_err(database)?;
        Ok(Value::Array(
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(database)?,
        ))
    }
}
use rusqlite::OptionalExtension;
/// Count only regular files; never follow a link outside the application's cache.
pub fn directory_bytes(root: &Path) -> u64 {
    fn visit(path: &Path, depth: u8) -> u64 {
        if depth > 4 {
            return 0;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .map(|e| {
                let Ok(kind) = e.file_type() else {
                    return 0;
                };
                if kind.is_file() {
                    e.metadata().map(|m| m.len()).unwrap_or(0)
                } else if kind.is_dir() {
                    visit(&e.path(), depth + 1)
                } else {
                    0
                }
            })
            .fold(0, u64::saturating_add)
    }
    visit(root, 0)
}
impl SenderHost {
    pub fn storage_status(&self) -> Result<Value> {
        let m = self.maintenance.lock().map_err(lock)?;
        let used = directory_bytes(&self.export_root) + directory_bytes(&self.request_root);
        let free = fs2::available_space(&self.request_root)?;
        // Leave room for one in-flight request and filesystem bookkeeping.
        let allowance = m
            .settings
            .cache_budget_bytes
            .saturating_sub(used)
            .min(free.saturating_sub(m.settings.min_free_bytes))
            .saturating_sub(8 << 20);
        let reason = if free <= m.settings.min_free_bytes + (8 << 20) {
            Some("local_free_space")
        } else if used + (8 << 20) >= m.settings.cache_budget_bytes {
            Some("local_cache_budget")
        } else {
            None
        };
        Ok(
            json!({"settings":m.settings,"used_bytes":used,"free_bytes":free,"export_allowance":allowance,"reason":reason}),
        )
    }
    fn reclaim_job(&self, job: &Job) -> Result<u64> {
        if job.state != photobridge_sender::JobState::Received {
            return Ok(0);
        }
        if fs::symlink_metadata(&self.export_root)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Ok(0);
        }
        let sender = self.sender.lock().map_err(lock)?;
        let Ok(root) = self.export_root.canonicalize() else {
            return Ok(0);
        };
        let mut removed = 0;
        for source in job.sources.values() {
            let p = Path::new(source);
            let Ok(metadata) = fs::symlink_metadata(p) else {
                continue;
            };
            if !metadata.file_type().is_file() || sender.source_in_use(source)? {
                continue;
            }
            let Ok(relative) = p.strip_prefix(&self.export_root) else {
                continue;
            };
            // Exports have exactly one app-created directory below exports/.
            if relative.components().count() != 2 {
                continue;
            }
            let Some(parent) = p.parent() else {
                continue;
            };
            if fs::symlink_metadata(parent)?.file_type().is_symlink() {
                continue;
            }
            let actual = p.canonicalize()?;
            if actual != root.join(relative) {
                continue;
            }
            fs::remove_file(p)?;
            removed += metadata.len();
            let _ = fs::remove_dir(parent);
        }
        drop(sender);
        if removed > 0 {
            self.maintenance.lock().map_err(lock)?.log(
                "cache_reclaimed",
                Some(job.id),
                Some(removed),
            )?;
        }
        Ok(removed)
    }
    pub fn reclaim_received(&self) -> Result<u64> {
        let mut cursor = 0;
        let mut total = 0;
        loop {
            let jobs = self.sender.lock().map_err(lock)?.list_filtered(
                cursor,
                200,
                None,
                Some("received"),
            )?;
            if jobs.is_empty() {
                break;
            }
            cursor = jobs.last().unwrap().id;
            for job in jobs {
                total += self.reclaim_job(&job)?;
            }
        }
        Ok(total)
    }
    pub fn record_result(&self, job: &Job) {
        let code = match job.state {
            photobridge_sender::JobState::Received => Some("transfer_received"),
            photobridge_sender::JobState::Waiting | photobridge_sender::JobState::Failed => {
                job.error_code.as_deref()
            }
            _ => None,
        };
        if let Some(code) = code {
            let _ = self
                .maintenance
                .lock()
                .map(|m| m.log(code, Some(job.id), Some(job.confirmed_bytes)));
        }
        let reclaim = self
            .maintenance
            .lock()
            .map(|m| m.settings.auto_reclaim)
            .unwrap_or(false);
        if reclaim && self.reclaim_job(job).is_err() {
            let _ = self
                .maintenance
                .lock()
                .map(|m| m.log("cache_reclaim_failed", Some(job.id), None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photobridge_sender::JobState;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    #[test]
    fn diagnostics_migrate_old_logs_and_reject_private_text() {
        let root = Temp::new();
        fs::create_dir_all(&root.0).unwrap();
        let conn = Connection::open(root.0.join("maintenance.sqlite3")).unwrap();
        conn.execute_batch("CREATE TABLE events(id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, code TEXT NOT NULL, job_id INTEGER, amount INTEGER);").unwrap();
        conn.execute(
            "INSERT INTO events(ts,code,job_id,amount) VALUES(?1,'transfer_received',14,42)",
            [now()],
        )
        .unwrap();
        drop(conn);
        let maintenance = Maintenance::open(&root.0).unwrap();
        let context: EventContext = serde_json::from_value(json!({
            "queued":12,"running":1,"waiting":3,"failed":0,"paused":false,
            "active_requests":1,"execution":"background","phase":"upload",
            "bytes_sent":4096,"bytes_expected":8192,"next_retry_at":now()+60
        }))
        .unwrap();
        maintenance
            .log_context("app_background", Some(15), None, Some(&context))
            .unwrap();
        drop(maintenance);
        // Reopening must be idempotent and preserve pre-migration entries.
        let entries = Maintenance::open(&root.0).unwrap().events().unwrap();
        assert_eq!(entries.as_array().unwrap().len(), 2);
        assert_eq!(entries[0]["context"]["queued"], 12);
        assert_eq!(entries[0]["context"]["bytes_expected"], 8192);
        assert_eq!(entries[1]["job_id"], 14);
        assert!(entries[1]["context"].is_null());
        for invalid in [
            json!({"token":"private"}),
            json!({"phase":"private-filename.jpg"}),
            json!({"queued":-1}),
            json!({"reason":"https://private.example"}),
        ] {
            assert!(serde_json::from_value::<EventContext>(invalid).is_err());
        }
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "photobridge-maintenance-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn job(host: &SenderHost, path: &Path, source: &str) -> Job {
        let bytes = fs::read(path).unwrap();
        let hash = digest(&bytes);
        let asset = Asset {
            version: 1,
            source_id: source.into(),
            revision: "1".into(),
            kind: AssetKind::Photo,
            metadata: BTreeMap::new(),
            resources: vec![Resource {
                role: ResourceRole::Photo,
                filename: "sample.jpg".into(),
                media_type: "image/jpeg".into(),
                size: bytes.len() as u64,
                sha256: hash.clone(),
            }],
        };
        host.enqueue(
            "receiver",
            asset,
            BTreeMap::from([(hash, path.to_str().unwrap().into())]),
        )
        .unwrap()
    }
    fn complete(host: &SenderHost, job: &Job) -> Job {
        let mut s = host.sender.lock().unwrap();
        let running = s.claim("receiver", now()).unwrap().unwrap();
        assert_eq!(running.id, job.id);
        s.acknowledge(
            &running.attempt(),
            &AssetStatus {
                asset_id: job.asset.id().unwrap(),
                receipt: ReceiptState::Received,
                processing: ProcessingState::NotRequested,
                resources: job
                    .asset
                    .resources
                    .iter()
                    .map(|r| ResourceStatus {
                        sha256: r.sha256.clone(),
                        offset: r.size,
                        complete: true,
                    })
                    .collect(),
            },
        )
        .unwrap();
        s.job(job.id).unwrap()
    }
    #[test]
    fn reclaim_requires_receipt_and_preserves_shared_sources_and_receipts() {
        let t = Temp::new();
        let host = SenderHost::open(&t.0.join("queue")).unwrap();
        let folder = t.0.join("exports/owned");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("photo.jpg");
        fs::write(&path, b"original").unwrap();
        let first = job(&host, &path, "one");
        let second = job(&host, &path, "two");
        assert_eq!(host.reclaim_job(&first).unwrap(), 0);
        let first = complete(&host, &first);
        assert_eq!(host.reclaim_job(&first).unwrap(), 0);
        assert!(path.exists());
        let second = complete(&host, &second);
        assert_eq!(host.reclaim_job(&second).unwrap(), 8);
        assert!(!path.exists());
        assert_eq!(
            host.sender.lock().unwrap().job(first.id).unwrap().state,
            JobState::Received
        );
        assert_eq!(host.reclaim_received().unwrap(), 0);
    }
    #[test]
    fn relocated_exports_recover_verified_failures_without_resuming_pause() {
        let t = Temp::new();
        let suffix = "Library/Application Support/PhotoBridge";
        let old = t.0.join("old").join(suffix);
        let host = SenderHost::open(&old.join("queue")).unwrap();
        let folder = old.join("exports/owned");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("photo.jpg");
        fs::write(&path, b"original").unwrap();
        let failed = job(&host, &path, "failed");
        let paused = job(&host, &path, "paused");
        {
            let mut sender = host.sender.lock().unwrap();
            let active = sender.claim("receiver", now()).unwrap().unwrap();
            assert_eq!(active.id, failed.id);
            sender
                .fail(&active.attempt(), Failure::SourceUnavailable, now())
                .unwrap();
            sender.pause_job(paused.id).unwrap();
            sender.set_paused(true).unwrap();
        }
        drop(host);
        fs::rename(t.0.join("old"), t.0.join("new")).unwrap();
        let new = t.0.join("new").join(suffix);
        let host = SenderHost::open(&new.join("queue")).unwrap();
        let sender = host.sender.lock().unwrap();
        let repaired = sender.job(failed.id).unwrap();
        assert_eq!(repaired.asset, failed.asset);
        assert_eq!(repaired.state, JobState::Queued);
        assert_eq!(repaired.error_code, None);
        assert_eq!(sender.job(paused.id).unwrap().state, JobState::Paused);
        assert!(sender.paused().unwrap());
        assert_eq!(
            Path::new(repaired.sources.values().next().unwrap()),
            new.join("exports/owned/photo.jpg")
        );
        drop(sender);
        drop(host);
        let reopened = SenderHost::open(&new.join("queue")).unwrap();
        assert_eq!(
            reopened
                .sender
                .lock()
                .unwrap()
                .job(failed.id)
                .unwrap()
                .sources,
            repaired.sources
        );
    }

    #[test]
    fn relocated_exports_reject_changed_and_linked_replacements() {
        let t = Temp::new();
        let suffix = "Library/Application Support/PhotoBridge";
        let old = t.0.join("old").join(suffix);
        let host = SenderHost::open(&old.join("queue")).unwrap();
        let folder = old.join("exports/owned");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("photo.jpg");
        fs::write(&path, b"original").unwrap();
        let original = job(&host, &path, "corrupt");
        drop(host);
        fs::rename(t.0.join("old"), t.0.join("new")).unwrap();
        let new = t.0.join("new").join(suffix);
        let replacement = new.join("exports/owned/photo.jpg");
        fs::write(&replacement, b"changed!").unwrap();
        let host = SenderHost::open(&new.join("queue")).unwrap();
        assert_eq!(
            host.sender
                .lock()
                .unwrap()
                .job(original.id)
                .unwrap()
                .sources,
            original.sources
        );
        drop(host);
        fs::remove_file(&replacement).unwrap();
        let external = t.0.join("external.jpg");
        fs::write(&external, b"original").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&external, &replacement).unwrap();
        let host = SenderHost::open(&new.join("queue")).unwrap();
        assert_eq!(
            host.sender
                .lock()
                .unwrap()
                .job(original.id)
                .unwrap()
                .sources,
            original.sources
        );
        assert!(external.exists());
    }

    #[test]
    fn relocated_motion_requires_both_original_components() {
        let t = Temp::new();
        let suffix = "Library/Application Support/PhotoBridge";
        let old = t.0.join("old").join(suffix);
        let host = SenderHost::open(&old.join("queue")).unwrap();
        let folder = old.join("exports/owned");
        fs::create_dir_all(&folder).unwrap();
        let photo = folder.join("photo.jpg");
        let video = folder.join("video.mov");
        fs::write(&photo, b"original").unwrap();
        fs::write(&video, b"paired video").unwrap();
        let first = job(&host, &photo, "motion");
        let mut asset = first.asset.clone();
        asset.kind = AssetKind::Motion;
        let hash = digest(b"paired video");
        asset.resources.push(Resource {
            role: ResourceRole::PairedVideo,
            filename: "video.mov".into(),
            media_type: "video/quicktime".into(),
            size: 12,
            sha256: hash.clone(),
        });
        let mut sources = first.sources;
        sources.insert(hash, video.to_str().unwrap().into());
        let motion = host.enqueue("receiver", asset, sources).unwrap();
        drop(host);
        fs::rename(t.0.join("old"), t.0.join("new")).unwrap();
        let new = t.0.join("new").join(suffix);
        let video = new.join("exports/owned/video.mov");
        fs::remove_file(&video).unwrap();
        let host = SenderHost::open(&new.join("queue")).unwrap();
        assert_eq!(
            host.sender.lock().unwrap().job(motion.id).unwrap().sources,
            motion.sources
        );
        drop(host);
        fs::write(&video, b"paired video").unwrap();
        let host = SenderHost::open(&new.join("queue")).unwrap();
        let restored = host.sender.lock().unwrap().job(motion.id).unwrap();
        assert_eq!(restored.asset, motion.asset);
        assert!(restored
            .sources
            .values()
            .all(|p| Path::new(p).starts_with(&new)));
    }
    #[test]
    fn reclaim_never_removes_external_or_linked_sources() {
        let t = Temp::new();
        let host = SenderHost::open(&t.0.join("queue")).unwrap();
        let external = t.0.join("original.jpg");
        fs::write(&external, b"original").unwrap();
        let j = job(&host, &external, "external");
        let j = complete(&host, &j);
        assert_eq!(host.reclaim_job(&j).unwrap(), 0);
        assert!(external.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            fs::create_dir_all(t.0.join("exports/owned")).unwrap();
            let linked = t.0.join("exports/owned/link.jpg");
            symlink(&external, &linked).unwrap();
            let j = job(&host, &linked, "link");
            let j = complete(&host, &j);
            assert_eq!(host.reclaim_job(&j).unwrap(), 0);
            assert!(linked.exists());
            let outside = t.0.join("outside");
            fs::create_dir_all(outside.join("child")).unwrap();
            let file = outside.join("child/file.jpg");
            fs::write(&file, b"data").unwrap();
            fs::remove_dir_all(t.0.join("exports")).unwrap();
            symlink(&outside, t.0.join("exports")).unwrap();
            let j = job(&host, &t.0.join("exports/child/file.jpg"), "root_link");
            let j = complete(&host, &j);
            assert_eq!(host.reclaim_job(&j).unwrap(), 0);
            assert!(file.exists());
        }
    }
    #[test]
    fn pending_sources_survive_restart_and_retry_without_crossing_receivers() {
        let t = Temp::new();
        {
            let m = Maintenance::open(&t.0).unwrap();
            m.schedule_sources("a", &["one".into(), "two".into(), "one".into()])
                .unwrap();
            m.schedule_sources("b", &["one".into()]).unwrap();
            m.source_result("a", "one", false).unwrap();
            m.source_result("a", "two", true).unwrap();
        }
        let mut m = Maintenance::open(&t.0).unwrap();
        assert_eq!(
            m.pending("a").unwrap(),
            json!({"count":1,"sources":[],"unordered":["one"]})
        );
        assert_eq!(m.pending("b").unwrap()["sources"], json!(["one"]));
        let inputs = vec![("one".into(), "1".into()), ("two".into(), "1".into())];
        assert_eq!(
            m.pending_states("a", &inputs)
                .unwrap()
                .get("one")
                .map(String::as_str),
            Some("preparing")
        );
        assert!(!m.pending_states("a", &inputs).unwrap().contains_key("two"));
        assert!(m.pending_states("other", &inputs).unwrap().is_empty());
        m.save(m.settings.clone()).unwrap();
        assert_eq!(m.pending("a").unwrap()["sources"], json!(["one"]));
    }
    #[test]
    fn source_browser_paginates_delayed_work_and_history_by_receiver() {
        let t = Temp::new();
        let m = Maintenance::open(&t.0).unwrap();
        let sources = (0..205).map(|n| format!("asset-{n}")).collect::<Vec<_>>();
        m.schedule_sources("a", &sources).unwrap();
        m.schedule_sources("b", &["private-b".into()]).unwrap();
        m.source_result("a", "asset-0", false).unwrap();
        let page = m.source_page("a", false, 0).unwrap();
        assert_eq!(page["total"], 205);
        assert_eq!(page["items"].as_array().unwrap().len(), 100);
        assert!(page["items"][0]["retry_at"].as_i64().unwrap() > now());
        let cursor = page["items"][99]["cursor"].as_i64().unwrap();
        m.source_result("a", "asset-0", true).unwrap();
        let second = m.source_page("a", false, cursor).unwrap();
        assert_eq!(second["items"][0]["source"], "asset-100");
        let end = m
            .source_page("a", false, second["items"][99]["cursor"].as_i64().unwrap())
            .unwrap();
        assert_eq!(end["items"].as_array().unwrap().len(), 5);
        assert_eq!(m.source_page("b", false, 0).unwrap()["total"], 1);
        let run = m.history_control("a", HistoryAction::Start).unwrap().run;
        m.history_batch(
            "a",
            run,
            &[
                ("received-photo".into(), "42".into()),
                ("new-photo".into(), "43".into()),
            ],
            &BTreeMap::from([("received-photo".into(), "received".into())]),
            true,
        )
        .unwrap();
        let history = m.source_page("a", true, 0).unwrap();
        assert_eq!(history["total"], 2);
        assert_eq!(history["items"][0]["revision"], "42");
        assert!(history["items"][0]["retry_at"].is_null());
        assert_eq!(history["items"][1]["retry_at"], 0);
        assert_eq!(m.source_page("b", true, 0).unwrap()["total"], 0);
        let next = m.history_control("a", HistoryAction::Start).unwrap().run;
        assert_ne!(run, next);
        assert_eq!(m.source_page("a", true, 0).unwrap()["total"], 0);
    }
    #[test]
    fn journal_is_bounded_private_and_settings_persist() {
        let t = Temp::new();
        {
            let mut m = Maintenance::open(&t.0).unwrap();
            let mut settings = m.settings.clone();
            settings.log_days = 1;
            settings.log_limit = 100;
            m.save(settings).unwrap();
            for _ in 0..110 {
                m.log("transfer_received", Some(7), Some(42)).unwrap();
            }
            assert!(m.log("/private/photo.jpg", None, None).is_err());
            assert_eq!(m.events().unwrap().as_array().unwrap().len(), 100);
            m.conn.execute("UPDATE events SET ts=0", []).unwrap();
            assert_eq!(m.events().unwrap(), json!([]));
        }
        let m = Maintenance::open(&t.0).unwrap();
        assert_eq!(m.settings.log_limit, 100);
        assert_eq!(m.settings.log_days, 1);
    }
}
