//! Durable local receiver storage. One writer owns the root; resources are
//! content-addressed, and an asset is received only after every resource verifies.
pub mod catalog;
pub mod devices;
pub mod retention;
pub mod usage;

use fs2::FileExt;
use photobridge_core::*;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

fn db(error: rusqlite::Error) -> Error {
    Error::Storage(error.to_string())
}
#[derive(serde::Serialize)]
pub struct Publication {
    pub id: String,
    pub asset: Asset,
    pub resources: std::collections::BTreeMap<String, PathBuf>,
    pub processing: ProcessingState,
}
pub struct Receiver {
    root: PathBuf,
    conn: Connection,
    capacity: u64,
    min_free: u64,
    admission_held: bool,
    _lock: File,
}
impl Receiver {
    pub fn observe_sender(
        &self,
        id: &str,
        ip: Option<std::net::IpAddr>,
        kind: Option<&str>,
    ) -> Result<()> {
        devices::DeviceDirectory::open(&self.root, "en")?.observe(id, ip, kind)
    }
    pub fn check_sender(&self, id: &str) -> Result<()> {
        if !devices::DeviceDirectory::open(&self.root, "en")?.enabled(id)? {
            return Err(Error::Conflict("sender reception paused".into()));
        }
        Ok(())
    }
    pub fn attribute_sender(&self, asset: &str, sender: &str) -> Result<()> {
        self.check_sender(sender)?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO asset_senders(asset_id,sender_id) VALUES(?1,?2)",
                params![asset, sender],
            )
            .map_err(db)?;
        Ok(())
    }
    pub fn exchange_device_profile(&self, peer: &DeviceProfile) -> Result<DeviceProfile> {
        peer.validate()?;
        let devices = devices::DeviceDirectory::open(&self.root, "en")?;
        devices.remember(&peer.id, peer)?;
        devices.profile()
    }
    pub fn open(root: impl AsRef<Path>, capacity: u64) -> Result<Self> {
        if capacity == 0 || capacity > i64::MAX as u64 {
            return Err(Error::Invalid("capacity".into()));
        }
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("blobs"))?;
        fs::create_dir_all(root.join("partial"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("receiver.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| Error::Conflict("receiver already open".into()))?;
        let conn = Connection::open(root.join("receiver.sqlite3")).map_err(db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS assets(id TEXT PRIMARY KEY, manifest TEXT NOT NULL, received INTEGER NOT NULL DEFAULT 0, processing TEXT NOT NULL DEFAULT 'not_requested');
            CREATE TABLE IF NOT EXISTS blobs(hash TEXT PRIMARY KEY, size INTEGER NOT NULL, ready INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS gallery_copies(asset_id TEXT PRIMARY KEY, copy TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS asset_senders(asset_id TEXT NOT NULL,sender_id TEXT NOT NULL,PRIMARY KEY(asset_id,sender_id));
            CREATE TABLE IF NOT EXISTS gallery_expected(asset_id TEXT PRIMARY KEY, copy TEXT NOT NULL);").map_err(db)?;
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(assets)")
            .map_err(db)?
            .query_map([], |r| r.get(1))
            .map_err(db)?
            .collect::<std::result::Result<_, _>>()
            .map_err(db)?;
        if !columns.iter().any(|c| c == "originals_released") {
            conn.execute(
                "ALTER TABLE assets ADD COLUMN originals_released INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(db)?;
        }
        if !columns.iter().any(|c| c == "release_reason") {
            conn.execute("ALTER TABLE assets ADD COLUMN release_reason TEXT", [])
                .map_err(db)?;
        }
        let mut receiver = Self {
            root,
            conn,
            capacity,
            min_free: 0,
            admission_held: false,
            _lock: lock,
        };
        receiver.reclaim_unreferenced()?;
        Ok(receiver)
    }
    pub fn hold_transfers(&mut self, held: bool) {
        self.admission_held = held;
    }
    fn check_admission(&self) -> Result<()> {
        if self.admission_held {
            return Err(Error::Conflict("receiver maintenance".into()));
        }
        Ok(())
    }
    pub fn configure_storage(&mut self, capacity: u64, min_free: u64) -> Result<()> {
        if capacity == 0 || capacity > i64::MAX as u64 || min_free > i64::MAX as u64 {
            return Err(Error::Invalid("storage limits".into()));
        }
        self.capacity = capacity;
        self.min_free = min_free;
        Ok(())
    }
    fn ensure_free(&self, additional: u64) -> Result<()> {
        if self.min_free > 0
            && fs2::available_space(&self.root)? < self.min_free.saturating_add(additional)
        {
            return Err(Error::LowSpace);
        }
        Ok(())
    }
    /// Local host-only publication view, never exposed by the network router.
    pub fn publications(&self, after: &str, limit: u32) -> Result<Vec<Publication>> {
        let mut stmt = self.conn.prepare("SELECT id FROM assets WHERE received=1 AND originals_released=0 AND processing!='complete' AND id>?1 ORDER BY id LIMIT ?2").map_err(db)?;
        let ids = stmt
            .query_map(params![after, limit.clamp(1, 100)], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        ids.into_iter()
            .map(|id| {
                let asset = self.asset(&id)?;
                let processing = self.status(&id)?.processing;
                let resources = asset
                    .resources
                    .iter()
                    .map(|r| (r.sha256.clone(), self.paths(&r.sha256).1))
                    .collect();
                Ok(Publication {
                    id,
                    asset,
                    resources,
                    processing,
                })
            })
            .collect()
    }
    pub fn originals_released(&self, id: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT originals_released FROM assets WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(db)
    }
    /// Stable, bounded original archive batch. Only fully published assets qualify.
    pub fn archive_batch(&self) -> Result<Vec<Publication>> {
        let mut query=self.conn.prepare("SELECT id FROM assets WHERE received=1 AND processing='complete' AND originals_released=0 ORDER BY id LIMIT 100").map_err(db)?;
        let ids = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        ids.into_iter()
            .map(|id| {
                let asset = self.asset(&id)?;
                let resources = asset
                    .resources
                    .iter()
                    .map(|r| (r.sha256.clone(), self.paths(&r.sha256).1))
                    .collect();
                Ok(Publication {
                    id,
                    asset,
                    resources,
                    processing: ProcessingState::Complete,
                })
            })
            .collect()
    }
    /// Platform adapters must freshly re-read the archive and supply its computed
    /// digests. This local-only operation never trusts a gallery publication alone.
    pub fn release_archived(
        &mut self,
        ids: &[String],
        verified: &std::collections::BTreeSet<String>,
    ) -> Result<u64> {
        if ids.is_empty() || ids.len() > 100 {
            return Err(Error::Invalid("archive batch".into()));
        }
        for id in ids {
            let status = self.status(id)?;
            if status.receipt != ReceiptState::Received
                || status.processing != ProcessingState::Complete
                || self
                    .asset(id)?
                    .resources
                    .iter()
                    .any(|r| !verified.contains(&r.sha256))
            {
                return Err(Error::Integrity);
            }
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        for id in ids {
            tx.execute("UPDATE assets SET originals_released=1,release_reason='archive' WHERE id=?1 AND originals_released=0", [id])
                .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        // Mark first; a crash only leaks reclaimable cache, never loses receipts.
        self.reclaim_unreferenced()
    }
    pub fn reclaim_unreferenced(&mut self) -> Result<u64> {
        let mut query=self.conn.prepare("SELECT hash,size FROM blobs WHERE NOT EXISTS (SELECT 1 FROM assets,json_each(assets.manifest,'$.resources') AS r WHERE originals_released=0 AND json_extract(r.value,'$.sha256')=blobs.hash)").map_err(db)?;
        let blobs = query
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
            })
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        drop(query);
        let mut freed = 0u64;
        for folder in ["blobs", "partial"] {
            if fs::symlink_metadata(self.root.join(folder))?
                .file_type()
                .is_symlink()
            {
                return Err(Error::Integrity);
            }
        }
        for (hash, size) in blobs {
            // Hash-addressed names only, never paths supplied by an archive.
            if !valid_digest(&hash) {
                return Err(Error::Integrity);
            }
            let (partial, complete) = self.paths(&hash);
            for path in [partial, complete] {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            self.conn
                .execute("DELETE FROM blobs WHERE hash=?1", [hash])
                .map_err(db)?;
            freed = freed.saturating_add(size);
        }
        Ok(freed)
    }
    /// Bounded, host-only receiver dashboard. Never returns source paths or credentials.
    pub fn overview(&self) -> Result<serde_json::Value> {
        let (total, received, published, failed): (i64,i64,i64,i64) = self.conn.query_row(
            "SELECT COUNT(*),COALESCE(SUM(received),0),COALESCE(SUM(processing='complete'),0),COALESCE(SUM(processing='failed'),0) FROM assets", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(db)?;
        let reserved: i64 = self
            .conn
            .query_row("SELECT COALESCE(SUM(size),0) FROM blobs", [], |r| r.get(0))
            .map_err(db)?;
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM assets ORDER BY received ASC,rowid DESC LIMIT 12")
            .map_err(db)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        let mut items = Vec::new();
        for id in ids {
            let asset = self.asset(&id)?;
            let status = self.status(&id)?;
            let bytes: u64 = asset.resources.iter().map(|r| r.size).sum();
            let confirmed: u64 = status.resources.iter().map(|r| r.offset).sum();
            items.push(serde_json::json!({"id":id,"filename":asset.resources[0].filename,"kind":asset.kind,"total_bytes":bytes,"confirmed_bytes":confirmed,"receipt":status.receipt,"processing":status.processing,"originals_released":self.originals_released(&id)?}));
        }
        Ok(
            serde_json::json!({"total":total,"received":received,"published":published,"failed":failed,"reserved_bytes":reserved,"capacity_bytes":self.capacity,"free_bytes":fs2::available_space(&self.root)?,"min_free_bytes":self.min_free,"recent":items}),
        )
    }
    pub fn register(&mut self, asset: Asset) -> Result<AssetStatus> {
        let id = asset.id()?;
        if let Ok(status) = self.status(&id) {
            if status.receipt == photobridge_core::ReceiptState::Received {
                return Ok(status);
            }
        }
        self.check_admission()?;
        let free = if self.min_free > 0 {
            Some(fs2::available_space(&self.root)?)
        } else {
            None
        };
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let mut reserved: i64 = tx
            .query_row("SELECT COALESCE(SUM(size), 0) FROM blobs", [], |r| r.get(0))
            .map_err(db)?;
        for r in &asset.resources {
            let size: Option<i64> = tx
                .query_row("SELECT size FROM blobs WHERE hash=?1", [&r.sha256], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(db)?;
            if let Some(size) = size {
                if size != r.size as i64 {
                    return Err(Error::Conflict("digest has a different size".into()));
                }
            } else {
                reserved = reserved.checked_add(r.size as i64).ok_or(Error::Capacity)?;
                if reserved > self.capacity as i64 {
                    return Err(Error::Capacity);
                }
                tx.execute(
                    "INSERT INTO blobs(hash,size) VALUES(?1,?2)",
                    params![r.sha256, r.size as i64],
                )
                .map_err(db)?;
            }
        }
        if let Some(free) = free {
            let pending: i64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(size),0) FROM blobs WHERE ready=0",
                    [],
                    |r| r.get(0),
                )
                .map_err(db)?;
            if free < self.min_free.saturating_add(pending as u64) {
                return Err(Error::LowSpace);
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO assets(id,manifest) VALUES(?1,?2)",
            params![id, serde_json::to_string(&asset)?],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        self.status(&id)
    }
    pub fn asset(&self, id: &str) -> Result<Asset> {
        if !valid_digest(id) {
            return Err(Error::Invalid("asset id".into()));
        }
        let value: Option<String> = self
            .conn
            .query_row("SELECT manifest FROM assets WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(db)?;
        Ok(serde_json::from_str(&value.ok_or(Error::NotFound)?)?)
    }
    fn paths(&self, hash: &str) -> (PathBuf, PathBuf) {
        (
            self.root.join("partial").join(hash),
            self.root.join("blobs").join(hash),
        )
    }
    fn resource_status(&self, r: &Resource) -> Result<ResourceStatus> {
        let ready: bool = self
            .conn
            .query_row(
                "SELECT ready FROM blobs WHERE hash=?1",
                [&r.sha256],
                |row| row.get(0),
            )
            .map_err(db)?;
        let (partial, complete) = self.paths(&r.sha256);
        if complete.exists() {
            if complete.metadata()?.len() != r.size {
                return Err(Error::Integrity);
            }
            // Recover a crash after file rename but before the database acknowledgement.
            if !ready {
                if digest_reader(File::open(&complete)?)? != r.sha256 {
                    return Err(Error::Integrity);
                }
                self.conn
                    .execute("UPDATE blobs SET ready=1 WHERE hash=?1", [&r.sha256])
                    .map_err(db)?;
            }
            return Ok(ResourceStatus {
                sha256: r.sha256.clone(),
                offset: r.size,
                complete: true,
            });
        }
        if ready {
            return Err(Error::Integrity);
        }
        let offset = if partial.exists() {
            partial.metadata()?.len()
        } else {
            0
        };
        if offset > r.size {
            return Err(Error::Integrity);
        }
        if offset == r.size {
            if digest_reader(File::open(&partial)?)? != r.sha256 {
                fs::remove_file(&partial)?;
                return Err(Error::Integrity);
            }
            fs::rename(&partial, &complete)?;
            #[cfg(unix)]
            File::open(self.root.join("blobs"))?.sync_all()?;
            self.conn
                .execute("UPDATE blobs SET ready=1 WHERE hash=?1", [&r.sha256])
                .map_err(db)?;
            return Ok(ResourceStatus {
                sha256: r.sha256.clone(),
                offset,
                complete: true,
            });
        }
        Ok(ResourceStatus {
            sha256: r.sha256.clone(),
            offset,
            complete: false,
        })
    }
    pub fn status(&self, id: &str) -> Result<AssetStatus> {
        let asset = self.asset(id)?;
        let (received, processing, released): (bool, String, bool) = self
            .conn
            .query_row(
                "SELECT received,processing,originals_released FROM assets WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(db)?;
        let processing = match processing.as_str() {
            "not_requested" => ProcessingState::NotRequested,
            "pending" => ProcessingState::Pending,
            "complete" => ProcessingState::Complete,
            "failed" => ProcessingState::Failed,
            _ => return Err(Error::Integrity),
        };
        Ok(AssetStatus {
            asset_id: id.into(),
            receipt: if received {
                ReceiptState::Received
            } else {
                ReceiptState::Receiving
            },
            processing,
            resources: asset
                .resources
                .iter()
                .map(|r| {
                    if received && released {
                        Ok(ResourceStatus {
                            sha256: r.sha256.clone(),
                            offset: r.size,
                            complete: true,
                        })
                    } else {
                        self.resource_status(r)
                    }
                })
                .collect::<Result<_>>()?,
        })
    }
    pub fn append(
        &mut self,
        id: &str,
        hash: &str,
        offset: u64,
        body: &[u8],
        chunk_hash: &str,
    ) -> Result<AssetStatus> {
        self.check_admission()?;
        if body.is_empty() || body.len() > MAX_CHUNK_BYTES {
            return Err(Error::Invalid("chunk size".into()));
        }
        if digest(body) != chunk_hash {
            return Err(Error::Integrity);
        }
        let asset = self.asset(id)?;
        let r = asset
            .resources
            .iter()
            .find(|r| r.sha256 == hash)
            .ok_or(Error::NotFound)?;
        let current = self.resource_status(r)?;
        let end = offset
            .checked_add(body.len() as u64)
            .ok_or_else(|| Error::Invalid("offset overflow".into()))?;
        if end > r.size {
            return Err(Error::Invalid("resource length".into()));
        }
        let (partial, complete) = self.paths(hash);
        if offset < current.offset {
            // Lost responses may cause exact chunk replays; contradictory writes fail.
            if end > current.offset {
                return Err(Error::Conflict("overlapping chunk".into()));
            }
            let mut file = File::open(if current.complete { complete } else { partial })?;
            file.seek(SeekFrom::Start(offset))?;
            let mut existing = vec![0; body.len()];
            file.read_exact(&mut existing)?;
            if existing != body {
                return Err(Error::Conflict("replayed chunk differs".into()));
            }
            return self.status(id);
        }
        if offset != current.offset || current.complete {
            return Err(Error::Conflict("offset mismatch".into()));
        }
        self.ensure_free(body.len() as u64)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(partial)?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(body)?;
        file.sync_all()?;
        drop(file);
        self.status(id)
    }
    pub fn commit(&mut self, id: &str) -> Result<AssetStatus> {
        let state = self.status(id)?;
        if self.originals_released(id)? {
            return Ok(state);
        }
        if state.resources.iter().any(|r| !r.complete) {
            return Err(Error::Conflict("asset has incomplete resources".into()));
        }
        for r in &self.asset(id)?.resources {
            if digest_reader(File::open(self.paths(&r.sha256).1)?)? != r.sha256 {
                return Err(Error::Integrity);
            }
        }
        self.conn
            .execute("UPDATE assets SET received=1 WHERE id=?1", [id])
            .map_err(db)?;
        self.status(id)
    }
    /// Local user action: retry target work without touching original receipts.
    pub fn retry_processing(&mut self) -> Result<usize> {
        self.conn
            .execute(
                "UPDATE assets SET processing='pending' WHERE received=1 AND processing='failed'",
                [],
            )
            .map_err(db)
    }
    /// Target work has its own state. Failure never rolls back the receipt.
    pub fn set_processing(&mut self, id: &str, next: ProcessingState) -> Result<AssetStatus> {
        let current = self.status(id)?;
        if current.receipt != ReceiptState::Received {
            return Err(Error::Conflict("asset not received".into()));
        }
        let valid = matches!(
            (&current.processing, &next),
            (ProcessingState::NotRequested, ProcessingState::Pending)
                | (
                    ProcessingState::Pending,
                    ProcessingState::Complete | ProcessingState::Failed
                )
                | (ProcessingState::Failed, ProcessingState::Pending)
        );
        if !valid && current.processing != next {
            return Err(Error::Conflict("processing transition".into()));
        }
        let value = match next {
            ProcessingState::NotRequested => "not_requested",
            ProcessingState::Pending => "pending",
            ProcessingState::Complete => "complete",
            ProcessingState::Failed => "failed",
        };
        self.conn
            .execute(
                "UPDATE assets SET processing=?2 WHERE id=?1",
                params![id, value],
            )
            .map_err(db)?;
        self.status(id)
    }
}
