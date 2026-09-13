//! Recover managed Apple exports when an app update relocates its data container.
use super::*;
use photobridge_sender::JobState;
use std::path::Component;

impl SenderHost {
    pub(super) fn restore_export_locations(&self) -> Result<()> {
        let Ok(root) = self.export_root.canonicalize() else {
            return Ok(());
        };
        let mut sender = self.sender.lock().map_err(lock)?;
        let mut after = 0;
        loop {
            let jobs = sender.list(after, 200)?;
            if jobs.is_empty() {
                break;
            }
            for job in jobs {
                after = job.id;
                if job.state == JobState::Received {
                    continue;
                }
                let mut sources = job.sources.clone();
                for path in sources.values_mut() {
                    if Path::new(path).exists() {
                        continue;
                    }
                    // Compare path components so native Windows separators and Apple
                    // POSIX separators follow the same narrow managed-export rule.
                    let parts: Vec<_> = Path::new(path).components().collect();
                    if parts.len() < 6 {
                        continue;
                    }
                    let tail = &parts[parts.len() - 6..];
                    let marker = ["Library", "Application Support", "PhotoBridge", "exports"];
                    if !tail[..4].iter().zip(marker).all(|(part, name)| {
                        matches!(part, Component::Normal(value) if *value == std::ffi::OsStr::new(name))
                    }) || !tail[4..].iter().all(|part| matches!(part, Component::Normal(_))) {
                        continue;
                    }
                    let relative = Path::new(tail[4].as_os_str()).join(tail[5].as_os_str());
                    let candidate = root.join(&relative);
                    // Never resolve a renamed export through a symlink or outside this store.
                    if candidate.canonicalize().ok().as_ref() != Some(&candidate)
                        || !fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_file())
                    {
                        continue;
                    }
                    *path = self
                        .export_root
                        .join(&relative)
                        .to_string_lossy()
                        .into_owned();
                }
                if sources == job.sources {
                    continue;
                }
                // Restore the whole asset only after every resource matches its manifest.
                // This also prevents a partial motion asset from being reported as repaired.
                let verified = job.asset.resources.iter().all(|resource| {
                    sources
                        .get(&resource.sha256)
                        .and_then(|p| File::open(p).ok())
                        .is_some_and(|file| {
                            file.metadata().is_ok_and(|m| m.len() == resource.size)
                                && digest_reader(file).is_ok_and(|hash| hash == resource.sha256)
                        })
                });
                if verified {
                    sender.rebind_sources(job.id, &sources)?;
                    self.maintenance.lock().map_err(lock)?.log(
                        "source_location_restored",
                        Some(job.id),
                        None,
                    )?;
                }
            }
        }
        Ok(())
    }
}
