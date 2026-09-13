//! Read-only, host-local history. Browsing never verifies, renames or deletes blobs.
use photobridge_core::{Asset, Error, Result};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
pub struct HistoryItem {
    pub cursor: i64,
    pub id: String,
    pub filename: String,
    pub kind: photobridge_core::AssetKind,
    pub total_bytes: u64,
    pub confirmed_bytes: u64,
    pub receipt: &'static str,
    pub processing: String,
    pub originals_released: bool,
    pub release_reason: Option<String>,
    pub senders: Vec<super::devices::Peer>,
}
#[derive(Serialize)]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
    pub total: i64,
    pub next_cursor: Option<i64>,
}
pub struct Catalog {
    root: PathBuf,
    conn: Option<Connection>,
}
impl Catalog {
    pub fn open(root: &Path) -> Result<Self> {
        let path = root.join("receiver.sqlite3");
        let conn = if path.exists() {
            let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(super::db)?;
            conn.busy_timeout(std::time::Duration::from_millis(250))
                .map_err(super::db)?;
            Some(conn)
        } else {
            None
        };
        Ok(Self {
            root: root.into(),
            conn,
        })
    }
    pub fn counts(&self) -> Result<serde_json::Value> {
        let Some(conn) = &self.conn else {
            return Ok(serde_json::json!({"total":0,"received":0,"published":0}));
        };
        let (total, received, published): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*),COALESCE(SUM(received),0),COALESCE(SUM(processing='complete'),0) FROM assets", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(super::db)?;
        Ok(serde_json::json!({"total":total,"received":received,"published":published}))
    }
    pub fn reserved_bytes(&self) -> Result<u64> {
        let Some(conn) = &self.conn else { return Ok(0) };
        conn.query_row("SELECT COALESCE(SUM(size),0) FROM blobs", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(super::db)
        .and_then(|v| u64::try_from(v).map_err(|_| Error::Integrity))
    }
    pub fn page(
        &self,
        before: Option<i64>,
        state: &str,
        kind: &str,
        limit: u32,
    ) -> Result<HistoryPage> {
        self.page_for_sender(before, state, kind, limit, None)
    }
    pub fn page_for_sender(
        &self,
        before: Option<i64>,
        state: &str,
        kind: &str,
        limit: u32,
        sender: Option<&str>,
    ) -> Result<HistoryPage> {
        if sender.is_some_and(|id| id != "unknown" && !photobridge_core::valid_digest(id)) {
            return Err(Error::Invalid("sender filter".into()));
        }
        if ![
            "all",
            "receiving",
            "received",
            "processing",
            "published",
            "failed",
        ]
        .contains(&state)
            || !["all", "photo", "video", "motion"].contains(&kind)
            || before.is_some_and(|v| v <= 0)
        {
            return Err(Error::Invalid("history filter".into()));
        }
        let Some(conn) = &self.conn else {
            return Ok(HistoryPage {
                items: vec![],
                total: 0,
                next_cursor: None,
            });
        };
        let has_senders: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_senders')",[],|r|r.get(0)).map_err(super::db)?;
        let sender_filter = if has_senders {
            "(?5 IS NULL OR (?5='unknown' AND NOT EXISTS(SELECT 1 FROM asset_senders s WHERE s.asset_id=assets.id)) OR EXISTS(SELECT 1 FROM asset_senders s WHERE s.asset_id=assets.id AND s.sender_id=?5))"
        } else {
            "(?5 IS NULL OR ?5='unknown')"
        };
        let filter = "(?1='all' OR (?1='receiving' AND received=0) OR (?1='received' AND received=1) OR (?1='published' AND processing='complete') OR (?1='failed' AND processing='failed') OR (?1='processing' AND received=1 AND processing IN ('pending','not_requested'))) AND (?2='all' OR json_extract(manifest,'$.kind')=?2)";
        let filter = format!("({filter}) AND {sender_filter}");
        let total = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM assets WHERE {filter}"),
                params![state, kind, Option::<i64>::None, 0, sender],
                |r| r.get(0),
            )
            .map_err(super::db)?;
        let limit = limit.clamp(1, 100) as usize;
        let has_reason: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('assets') WHERE name='release_reason')", [], |r| r.get(0)).map_err(super::db)?;
        let reason_column = if has_reason { "release_reason" } else { "NULL" };
        let mut query = conn.prepare(&format!("SELECT rowid,id,manifest,received,processing,originals_released,{reason_column} FROM assets WHERE {filter} AND (?3 IS NULL OR rowid<?3) ORDER BY rowid DESC LIMIT ?4")).map_err(super::db)?;
        let records = query
            .query_map(
                params![state, kind, before, limit as i64 + 1, sender],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, bool>(5)?,
                        r.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .map_err(super::db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(super::db)?;
        let more = records.len() > limit;
        let mut items = Vec::new();
        let peers = super::devices::DeviceDirectory::open(&self.root, "en")?.peers()?;
        for (cursor, id, manifest, received, processing, originals_released, release_reason) in
            records.into_iter().take(limit)
        {
            let asset: Asset = serde_json::from_str(&manifest)?;
            asset.validate()?;
            let total_bytes = asset.resources.iter().map(|r| r.size).sum();
            let confirmed_bytes = if received {
                total_bytes
            } else {
                asset
                    .resources
                    .iter()
                    .map(|r| {
                        let complete = self.root.join("blobs").join(&r.sha256);
                        let partial = self.root.join("partial").join(&r.sha256);
                        fs::metadata(complete)
                            .or_else(|_| fs::metadata(partial))
                            .map(|m| m.len().min(r.size))
                            .unwrap_or(0)
                    })
                    .sum()
            };
            let sender_ids = if has_senders {
                let mut q = conn
                    .prepare(
                        "SELECT sender_id FROM asset_senders WHERE asset_id=?1 ORDER BY sender_id",
                    )
                    .map_err(super::db)?;
                let ids = q
                    .query_map([&id], |r| r.get::<_, String>(0))
                    .map_err(super::db)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(super::db)?;
                ids
            } else {
                vec![]
            };
            let senders = peers
                .iter()
                .filter(|p| sender_ids.contains(&p.profile.id))
                .cloned()
                .collect();
            items.push(HistoryItem {
                cursor,
                id,
                filename: asset.resources[0].filename.clone(),
                kind: asset.kind,
                total_bytes,
                confirmed_bytes,
                receipt: if received { "received" } else { "receiving" },
                processing,
                originals_released,
                release_reason,
                senders,
            });
        }
        let next_cursor = if more {
            items.last().map(|i| i.cursor)
        } else {
            None
        };
        Ok(HistoryPage {
            items,
            total,
            next_cursor,
        })
    }
}
