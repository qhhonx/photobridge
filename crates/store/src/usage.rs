//! Read-only file-size snapshot, separate from reserved transfer capacity.
//! Concurrent writes may change the snapshot. Never acquire the receiver writer
//! or hash originals merely to render a storage page.
use photobridge_core::{Error, Result};
use serde::Serialize;
use std::{collections::HashMap, fs, io, path::Path};

#[derive(Default, Serialize)]
pub struct OriginalUsage {
    pub ready_bytes: u64,
    pub ready_files: u64,
    pub partial_bytes: u64,
    pub partial_files: u64,
}
fn entries(
    path: &Path,
    mut visit: impl FnMut(std::ffi::OsString, u64) -> Result<()>,
) -> Result<()> {
    let directory = match fs::read_dir(path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in directory {
        let entry = entry?;
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if metadata.is_file() {
            visit(entry.file_name(), metadata.len())?;
        }
    }
    Ok(())
}
pub fn original_usage(store: &Path) -> Result<OriginalUsage> {
    // Windows reports a missing child below a regular file as NotFound too.
    // Distinguish a missing store from an invalid store before enumerating it.
    match fs::symlink_metadata(store) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "store is not a directory").into(),
            )
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(OriginalUsage::default())
        }
        Err(error) => return Err(error.into()),
    }
    let mut pending = HashMap::new();
    entries(&store.join("partial"), |name, bytes| {
        pending.insert(name, bytes);
        Ok(())
    })?;
    let mut usage = OriginalUsage::default();
    entries(&store.join("blobs"), |name, bytes| {
        // A completion rename during enumeration must not count a blob twice.
        pending.remove(&name);
        usage.ready_bytes = usage
            .ready_bytes
            .checked_add(bytes)
            .ok_or(Error::Capacity)?;
        usage.ready_files += 1;
        Ok(())
    })?;
    usage.partial_files = pending.len() as u64;
    for bytes in pending.into_values() {
        usage.partial_bytes = usage
            .partial_bytes
            .checked_add(bytes)
            .ok_or(Error::Capacity)?;
    }
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_sizes_are_not_reservations_and_reclamation_updates_the_snapshot() {
        let root = std::env::temp_dir().join(format!("photobridge-usage-{}", std::process::id()));
        fs::create_dir_all(root.join("blobs/nested")).unwrap();
        fs::create_dir_all(root.join("partial")).unwrap();
        fs::write(root.join("blobs/a"), [0; 7]).unwrap();
        fs::write(root.join("partial/b"), [0; 3]).unwrap();
        fs::write(root.join("partial/a"), [0; 7]).unwrap();
        fs::write(root.join("blobs/nested/not-a-blob"), [0; 99]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("blobs/a"), root.join("blobs/link")).unwrap();
        let usage = original_usage(&root).unwrap();
        assert_eq!((usage.ready_bytes, usage.ready_files), (7, 1));
        assert_eq!((usage.partial_bytes, usage.partial_files), (3, 1));
        fs::remove_file(root.join("partial/a")).unwrap();
        fs::remove_file(root.join("blobs/a")).unwrap();
        assert_eq!(original_usage(&root).unwrap().ready_bytes, 0);
        fs::write(root.join("bad-directory"), [0; 1]).unwrap();
        assert!(original_usage(&root.join("bad-directory")).is_err());
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(original_usage(&root).unwrap().ready_bytes, 0);
        assert!(!root.exists());
    }
}
