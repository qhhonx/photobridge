//! Host-local delivery evidence. This is not a cloud receipt or original archive.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryCopy {
    pub locator: String,
    pub sha256: String,
    pub size: u64,
}
impl GalleryCopy {
    fn validate(&self) -> Result<()> {
        if self.locator.is_empty()
            || self.locator.len() > 2048
            || self.locator.chars().any(char::is_control)
            || !valid_digest(&self.sha256)
            || self.size == 0
            || self.size > i64::MAX as u64
        {
            return Err(Error::Invalid("gallery copy".into()));
        }
        Ok(())
    }
}
#[derive(Serialize)]
pub struct GalleryCandidate {
    pub publication: Publication,
    pub copy: Option<GalleryCopy>,
    pub confirmed: bool,
}
impl Receiver {
    pub fn expected_gallery_copy(&self, id: &str) -> Result<Option<GalleryCopy>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT copy FROM gallery_expected WHERE asset_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Error::from))
            .transpose()
    }
    pub fn prepare_gallery_copy(&mut self, id: &str, copy: &GalleryCopy) -> Result<()> {
        copy.validate()?;
        let eligible: bool = self
            .conn
            .query_row(
                "SELECT received=1 AND originals_released=0 FROM assets WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !eligible {
            return Err(Error::Integrity);
        }
        if self.gallery_copy(id)?.is_some_and(|old| old != *copy) {
            return Err(Error::Integrity);
        }
        // Only the host's owned pending item may be replaced. Complete copies
        // are immutable and must be read back against existing evidence.
        self.conn.execute("INSERT INTO gallery_expected(asset_id,copy) VALUES(?1,?2) ON CONFLICT(asset_id) DO UPDATE SET copy=excluded.copy", params![id,serde_json::to_string(copy)?]).map_err(db)?;
        Ok(())
    }
    pub fn gallery_copy(&self, id: &str) -> Result<Option<GalleryCopy>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT copy FROM gallery_copies WHERE asset_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Error::from))
            .transpose()
    }
    /// The native adapter freshly verifies complete delivery bytes before calling.
    /// Record the immutable evidence and publication completion atomically.
    pub fn record_gallery_copy(&mut self, id: &str, copy: &GalleryCopy) -> Result<()> {
        copy.validate()?;
        let received: bool = self
            .conn
            .query_row("SELECT received FROM assets WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(db)?;
        if !received {
            return Err(Error::Integrity);
        }
        if self.gallery_copy(id)?.is_some_and(|old| old != *copy) {
            return Err(Error::Conflict("gallery copy changed".into()));
        }
        if self
            .expected_gallery_copy(id)?
            .is_some_and(|old| old != *copy)
        {
            return Err(Error::Integrity);
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute(
            "INSERT OR IGNORE INTO gallery_copies(asset_id,copy) VALUES(?1,?2)",
            params![id, serde_json::to_string(copy)?],
        )
        .map_err(db)?;
        tx.execute("DELETE FROM gallery_expected WHERE asset_id=?1", [id])
            .map_err(db)?;
        tx.execute("UPDATE assets SET processing='complete' WHERE id=?1", [id])
            .map_err(db)?;
        tx.commit().map_err(db)
    }
    /// Independent opt-in relay policy; never pass gallery evidence to archive
    /// reclamation. The fresh proof must exactly match recorded delivery bytes.
    pub fn release_gallery_copy(
        &mut self,
        id: &str,
        verified: &GalleryCopy,
        relay_enabled: bool,
    ) -> Result<u64> {
        if !relay_enabled {
            return Err(Error::Conflict("relay disabled".into()));
        }
        verified.validate()?;
        if self.gallery_copy(id)?.as_ref() != Some(verified) {
            return Err(Error::Integrity);
        }
        let eligible: bool = self
            .conn
            .query_row(
                "SELECT received=1 AND processing='complete' FROM assets WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !eligible {
            return Err(Error::Integrity);
        }
        // Mark before deleting, preserving receipts and recovery after a crash.
        self.conn.execute("UPDATE assets SET originals_released=1,release_reason='gallery' WHERE id=?1 AND originals_released=0",[id]).map_err(db)?;
        self.reclaim_unreferenced()
    }
    pub fn gallery_candidates(&self, after: &str) -> Result<Vec<GalleryCandidate>> {
        if !after.is_empty() && !valid_digest(after) {
            return Err(Error::Invalid("gallery cursor".into()));
        }
        let mut query = self.conn.prepare("SELECT id FROM assets WHERE received=1 AND processing='complete' AND originals_released=0 AND id>?1 ORDER BY id LIMIT 4").map_err(db)?;
        let ids = query
            .query_map([after], |r| r.get::<_, String>(0))
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
                let confirmed_copy = self.gallery_copy(&id)?;
                let confirmed = confirmed_copy.is_some();
                let copy = confirmed_copy.or(self.expected_gallery_copy(&id)?);
                Ok(GalleryCandidate {
                    publication: Publication {
                        id,
                        asset,
                        resources,
                        processing: ProcessingState::Complete,
                    },
                    copy,
                    confirmed,
                })
            })
            .collect()
    }
}
