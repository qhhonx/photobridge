//! Platform-independent asset contracts and durable-transfer decisions.
//! No runtime, UI, operating-system scheduler, or Google Photos dependencies.
mod bundle;
pub use bundle::{bundle_size, write_bundle, BUNDLE_CONTENT_TYPE, BUNDLE_MAGIC};
mod burst;
pub use burst::BurstMetadata;
mod device;
pub use device::DeviceProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_CHUNK_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("resource not found")]
    NotFound,
    #[error("receiver capacity exceeded")]
    Capacity,
    #[error("receiver free space is below reserve")]
    LowSpace,
    #[error("integrity verification failed")]
    Integrity,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("receiver authentication required")]
    Unauthorized,
    #[error("operation cancelled")]
    Cancelled,
    #[error("unsupported capability: {0}")]
    Unsupported(String),
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(error.to_string())
    }
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn digest_reader(mut reader: impl Read) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Photo,
    Video,
    Motion,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceRole {
    Photo,
    Video,
    PairedVideo,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub role: ResourceRole,
    pub filename: String,
    pub media_type: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub version: u32,
    pub source_id: String,
    /// A source-provided revision; content digests remain authoritative.
    pub revision: String,
    pub kind: AssetKind,
    pub metadata: BTreeMap<String, String>,
    pub resources: Vec<Resource>,
}
impl Asset {
    pub fn validate(&self) -> Result<()> {
        if self.version != PROTOCOL_VERSION {
            return Err(Error::Unsupported("protocol version".into()));
        }
        if self.source_id.is_empty()
            || self.source_id.len() > 1024
            || self.revision.len() > 256
            || serde_json::to_vec(self)?.len() > MAX_MANIFEST_BYTES
        {
            return Err(Error::Invalid("manifest limits".into()));
        }
        if BurstMetadata::from_fields(&self.metadata)?.is_some() && self.kind == AssetKind::Video {
            return Err(Error::Invalid("burst video relationship".into()));
        }
        let roles: Vec<_> = self.resources.iter().map(|r| &r.role).collect();
        let shape = match self.kind {
            AssetKind::Photo => roles == [&ResourceRole::Photo],
            AssetKind::Video => roles == [&ResourceRole::Video],
            AssetKind::Motion => roles == [&ResourceRole::Photo, &ResourceRole::PairedVideo],
        };
        if !shape {
            return Err(Error::Invalid("asset resource structure".into()));
        }
        let mut hashes = BTreeSet::new();
        for r in &self.resources {
            if !valid_digest(&r.sha256)
                || !hashes.insert(&r.sha256)
                || r.size == 0
                || r.size > i64::MAX as u64
                || r.filename.is_empty()
                || r.filename.len() > 255
                || r.filename.contains(['/', '\\'])
                || r.filename.chars().any(char::is_control)
                || r.filename == "."
                || r.filename == ".."
                || r.media_type.is_empty()
                || r.media_type.len() > 128
                || r.media_type.chars().any(char::is_control)
            {
                return Err(Error::Invalid("resource descriptor".into()));
            }
        }
        Ok(())
    }
    /// Metadata uses a sorted map, so native bindings can reproduce this identity.
    pub fn id(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(&serde_json::to_vec(self)?))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    Receiving,
    Received,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingState {
    NotRequested,
    Pending,
    Complete,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceStatus {
    pub sha256: String,
    pub offset: u64,
    pub complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetStatus {
    pub asset_id: String,
    pub receipt: ReceiptState,
    pub processing: ProcessingState,
    pub resources: Vec<ResourceStatus>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub version: u32,
    pub max_chunk_bytes: usize,
    pub motion_assets: bool,
    pub target_processing: Vec<String>,
    #[serde(default)]
    pub bundle_upload: bool,
}
impl Default for Capabilities {
    fn default() -> Self {
        Self {
            version: PROTOCOL_VERSION,
            max_chunk_bytes: MAX_CHUNK_BYTES,
            motion_assets: true,
            target_processing: vec![],
            bundle_upload: true,
        }
    }
}
/// A durable plan can be handed to URLSession or any other native executor.
/// It contains no authentication secret and does not require a live Rust Future.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TransferAction {
    Upload {
        sha256: String,
        offset: u64,
        length: u64,
    },
    Commit,
    Done,
}
pub fn next_action(
    asset: &Asset,
    status: &AssetStatus,
    chunk_bytes: usize,
) -> Result<TransferAction> {
    if status.asset_id != asset.id()?
        || chunk_bytes == 0
        || chunk_bytes > MAX_CHUNK_BYTES
        || status.resources.len() != asset.resources.len()
    {
        return Err(Error::Invalid("receiver status".into()));
    }
    let mut pending = None;
    for (resource, remote) in asset.resources.iter().zip(&status.resources) {
        if resource.sha256 != remote.sha256
            || remote.offset > resource.size
            || (remote.complete && remote.offset != resource.size)
        {
            return Err(Error::Invalid("resource status".into()));
        }
        if !remote.complete {
            if status.receipt == ReceiptState::Received || remote.offset == resource.size {
                return Err(Error::Integrity);
            }
            pending.get_or_insert(TransferAction::Upload {
                sha256: resource.sha256.clone(),
                offset: remote.offset,
                length: (resource.size - remote.offset).min(chunk_bytes as u64),
            });
        }
    }
    if let Some(action) = pending {
        return Ok(action);
    }
    Ok(if status.receipt == ReceiptState::Received {
        TransferAction::Done
    } else {
        TransferAction::Commit
    })
}
/// Scheduling and photo library enumeration belong to the native host.
pub trait AssetSource {
    fn assets_since(&mut self, cursor: Option<&str>) -> Result<(Vec<Asset>, Option<String>)>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetPlan {
    PublishOriginals,
    GenerateBurstPhoto,
    GenerateMotionPhoto,
}
pub trait TargetProcessor {
    fn plan(&self, asset: &Asset) -> Result<TargetPlan>;
}
