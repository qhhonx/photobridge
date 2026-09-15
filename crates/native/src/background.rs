//! Request planning for file-backed native background transfers. No networking
//! or platform scheduling here: each OS operation is fenced by a queue attempt.
use super::*;
use photobridge_sender::Attempt;
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct NativeRequest {
    pub attempt: Attempt,
    pub method: String,
    pub path: String,
    pub content_type: String,
    pub body_file: PathBuf,
    pub transfer_mode: &'static str,
}
impl SenderHost {
    pub fn set_bundle_upload(&self, receiver: &str, enabled: bool) -> Result<()> {
        self.sender
            .lock()
            .map_err(lock)?
            .set_bundle_upload(receiver, enabled)
    }
    fn request_file(&self, a: &Attempt) -> PathBuf {
        self.request_root
            .join(format!("{}-{}.body", a.job_id, a.generation))
    }
    pub fn prepare_native(&self, receiver: &str) -> Result<Option<NativeRequest>> {
        let mut sender = self.sender.lock().map_err(lock)?;
        let Some(job) = sender.claim_native(receiver, now())? else {
            return Ok(None);
        };
        let attempt = job.attempt();
        let result = (|| {
            let checkpoint = sender.checkpoint(job.id)?;
            let id = job.asset.id()?;
            // Continue any already-started legacy sequence. If staging another
            // original would exceed the cache budget, use the bounded chunk path.
            let bundle_enabled = sender.bundle_upload(receiver)?;
            if checkpoint.is_none() && bundle_enabled {
                let bytes = bundle_size(&job.asset)?;
                let allowance = self.storage_status()?["export_allowance"]
                    .as_u64()
                    .unwrap_or(0);
                if bytes <= allowance {
                    let body_file = self.request_file(&attempt);
                    let mut options = OpenOptions::new();
                    options.write(true).create_new(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600);
                    }
                    let mut output = options.open(&body_file)?;
                    write_bundle(&job.asset, &mut output, |resource| {
                        let source = job.sources.get(&resource.sha256).ok_or(Error::NotFound)?;
                        Ok(Box::new(File::open(source)?))
                    })?;
                    output.sync_all()?;
                    return Ok(NativeRequest {
                        attempt: attempt.clone(),
                        method: "POST".into(),
                        path: "/v1/bundles".into(),
                        content_type: BUNDLE_CONTENT_TYPE.into(),
                        body_file,
                        transfer_mode: "bundle",
                    });
                }
            }
            let transfer_mode = if checkpoint.is_some() {
                "resume_chunks"
            } else if bundle_enabled {
                "cache_limited_chunks"
            } else {
                "legacy_chunks"
            };
            let (method, path, content_type, body) = match checkpoint {
                None => (
                    "POST",
                    "/v1/assets".into(),
                    "application/json",
                    serde_json::to_vec(&job.asset)?,
                ),
                Some(status) => {
                    match next_action(&job.asset, &status, MAX_CHUNK_BYTES)? {
                        TransferAction::Upload {
                            sha256,
                            offset,
                            length,
                        } => {
                            let source = job.sources.get(&sha256).ok_or(Error::NotFound)?;
                            let mut file = File::open(source)?;
                            let resource = job
                                .asset
                                .resources
                                .iter()
                                .find(|r| r.sha256 == sha256)
                                .ok_or(Error::Integrity)?;
                            if file.metadata()?.len() != resource.size {
                                return Err(Error::Integrity);
                            }
                            file.seek(SeekFrom::Start(offset))?;
                            let mut bytes = vec![0; length as usize];
                            file.read_exact(&mut bytes)?;
                            let hash = digest(&bytes);
                            ("PUT", format!("/v1/assets/{id}/resources/{sha256}?offset={offset}&sha256={hash}"), "application/octet-stream", bytes)
                        }
                        TransferAction::Commit => (
                            "POST",
                            format!("/v1/assets/{id}/commit"),
                            "application/json",
                            b"{}".to_vec(),
                        ),
                        TransferAction::Done => {
                            return Err(Error::Conflict("received checkpoint on queued job".into()))
                        }
                    }
                }
            };
            let body_file = self.request_file(&attempt);
            private_write(&body_file, &body)?;
            Ok(NativeRequest {
                attempt: attempt.clone(),
                method: method.into(),
                path,
                content_type: content_type.into(),
                body_file,
                transfer_mode,
            })
        })();
        match result {
            Ok(request) => Ok(Some(request)),
            Err(error) => {
                sender.fail(&attempt, Failure::from_error(&error), now())?;
                let _ = fs::remove_file(self.request_file(&attempt));
                Err(error)
            }
        }
    }
    pub fn bind_native(&self, attempt: &Attempt, task_id: &str) -> Result<()> {
        self.sender
            .lock()
            .map_err(lock)?
            .bind_native_task(attempt, task_id)
    }
    pub fn abandon_native(&self, attempt: &Attempt) -> Result<()> {
        // Used when the host cannot construct an OS request, before resume().
        let mut s = self.sender.lock().map_err(lock)?;
        s.interrupted(attempt)?;
        let _ = fs::remove_file(self.request_file(attempt));
        Ok(())
    }
    pub fn finish_native(
        &self,
        attempt: &Attempt,
        task_id: &str,
        status_code: u16,
        body: &str,
        failure: Option<Failure>,
        cancelled: bool,
    ) -> Result<Job> {
        let mut sender = self.sender.lock().map_err(lock)?;
        sender.check_native(attempt, task_id)?;
        if cancelled {
            sender.interrupted(attempt)?;
        } else if let Some(failure) = failure {
            sender.fail(attempt, failure, now())?;
        } else if (200..300).contains(&status_code) {
            let acknowledged = serde_json::from_str::<AssetStatus>(body)
                .map_err(|_| Error::Integrity)
                .and_then(|status| sender.complete_native(attempt, task_id, &status));
            if acknowledged.is_err() {
                sender.fail(attempt, Failure::Integrity, now())?;
            }
        } else {
            let failure = match status_code {
                401 | 403 => Failure::Authentication,
                409 => Failure::Busy,
                507 if serde_json::from_str::<Value>(body)
                    .ok()
                    .is_some_and(|v| v["code"] == "low_space") =>
                {
                    Failure::LowSpace
                }
                507 => Failure::Capacity,
                422 => Failure::Integrity,
                400 | 405 | 413 | 415 => Failure::Unsupported,
                _ => Failure::Network,
            };
            sender.fail(attempt, failure, now())?;
        }
        let _ = fs::remove_file(self.request_file(attempt));
        let job = sender.job(attempt.job_id)?;
        drop(sender);
        self.record_result(&job);
        Ok(job)
    }
    pub fn reconcile_native(&self, live: &BTreeSet<String>) -> Result<Vec<String>> {
        let sender = &mut *self.sender.lock().map_err(lock)?;
        let cancel = sender.reconcile_native(live)?;
        // Request files are immutable and named by generation. Retain only files
        // still owned by an OS task; its cancellation callback performs cleanup.
        for entry in fs::read_dir(&self.request_root)? {
            let path = entry?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some((id, generation)) = stem.split_once('-') else {
                continue;
            };
            let (Ok(id), Ok(generation)) = (id.parse::<i64>(), generation.parse::<i64>()) else {
                continue;
            };
            let keep = sender
                .job(id)
                .is_ok_and(|j| j.generation == generation && j.native_task_id.is_some());
            if !keep {
                let _ = fs::remove_file(path);
            }
        }
        Ok(cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_window_pause_budget_and_source_integrity() {
        let root =
            std::env::temp_dir().join(format!("photobridge-bundle-planner-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let host = SenderHost::open(&root.join("queue")).unwrap();
        let photo = root.join("photo.jpg");
        fs::write(&photo, b"0123456789").unwrap();
        let asset = Asset {
            version: 1,
            source_id: "bundle".into(),
            revision: "1".into(),
            kind: AssetKind::Photo,
            metadata: BTreeMap::new(),
            resources: vec![Resource {
                role: ResourceRole::Photo,
                filename: "photo.jpg".into(),
                media_type: "image/jpeg".into(),
                size: 10,
                sha256: digest(b"0123456789"),
            }],
        };
        let sources = BTreeMap::from([(
            asset.resources[0].sha256.clone(),
            photo.to_str().unwrap().into(),
        )]);
        for i in 0..6 {
            let mut asset = asset.clone();
            asset.source_id = format!("bundle-{i}");
            host.enqueue("peer", asset, sources.clone()).unwrap();
        }
        host.set_bundle_upload("peer", true).unwrap();
        let mut planned = Vec::new();
        for _ in 0..4 {
            let request = host.prepare_native("peer").unwrap().unwrap();
            assert_eq!(request.path, "/v1/bundles");
            assert!(fs::read(&request.body_file)
                .unwrap()
                .starts_with(BUNDLE_MAGIC));
            host.bind_native(&request.attempt, &request.attempt.job_id.to_string())
                .unwrap();
            planned.push(request);
        }
        assert!(host.prepare_native("peer").unwrap().is_none());
        host.pause(true).unwrap();
        for request in &planned {
            // Pause preserves immutable files until the OS confirms cancellation.
            assert!(request.body_file.exists());
            host.finish_native(
                &request.attempt,
                &request.attempt.job_id.to_string(),
                0,
                "",
                None,
                true,
            )
            .unwrap();
            assert!(!request.body_file.exists());
        }
        assert!(host.prepare_native("peer").unwrap().is_none());
        host.pause(false).unwrap();
        host.maintenance.lock().unwrap().settings.cache_budget_bytes = 1;
        let legacy = host.prepare_native("peer").unwrap().unwrap();
        assert_eq!(legacy.path, "/v1/assets");
        host.abandon_native(&legacy.attempt).unwrap();
        host.maintenance.lock().unwrap().settings.cache_budget_bytes = 5 << 30;
        fs::write(&photo, b"corrupted!").unwrap();
        assert!(matches!(host.prepare_native("peer"), Err(Error::Integrity)));
        assert_eq!(fs::read_dir(&host.request_root).unwrap().count(), 0);
        drop(host);
        let restored = SenderHost::open(&root.join("queue")).unwrap();
        assert!(restored
            .sender
            .lock()
            .unwrap()
            .bundle_upload("peer")
            .unwrap());
        drop(restored);
        fs::remove_dir_all(root).unwrap();
    }
}
