//! File-backed single-request envelope. It keeps original bytes and the existing
//! asset identity; it is not an image conversion or an archive retention format.
use super::*;
use std::io::{Read, Write};
pub const BUNDLE_MAGIC: &[u8; 8] = b"PBRG0001";
pub const BUNDLE_CONTENT_TYPE: &str = "application/x-photobridge-bundle";
pub fn bundle_size(asset: &Asset) -> Result<u64> {
    asset.validate()?;
    let manifest = serde_json::to_vec(asset)?;
    asset
        .resources
        .iter()
        .try_fold(12 + manifest.len() as u64, |sum, r| {
            sum.checked_add(r.size).ok_or(Error::Capacity)
        })
}
/// The caller owns staging-space reservations and removes incomplete output on
/// error. Reads are bounded and every original digest is verified during copying.
pub fn write_bundle<W: Write>(
    asset: &Asset,
    mut output: W,
    mut source: impl FnMut(&Resource) -> Result<Box<dyn Read>>,
) -> Result<u64> {
    let total = bundle_size(asset)?;
    let manifest = serde_json::to_vec(asset)?;
    output.write_all(BUNDLE_MAGIC)?;
    output.write_all(&(manifest.len() as u32).to_be_bytes())?;
    output.write_all(&manifest)?;
    let mut buffer = [0u8; 64 * 1024];
    for resource in &asset.resources {
        let mut input = source(resource)?;
        let mut remaining = resource.size;
        let mut hash = Sha256::new();
        while remaining > 0 {
            let length = remaining.min(buffer.len() as u64) as usize;
            input.read_exact(&mut buffer[..length])?;
            output.write_all(&buffer[..length])?;
            hash.update(&buffer[..length]);
            remaining -= length as u64;
        }
        if input.read(&mut buffer[..1])? != 0 || format!("{:x}", hash.finalize()) != resource.sha256
        {
            return Err(Error::Integrity);
        }
    }
    output.flush()?;
    Ok(total)
}
