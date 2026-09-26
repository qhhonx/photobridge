//! Optional desktop boundary. Source files are never owned caches.
use super::*;
use photobridge_folder_source::{resolve, revision, Index};
static INDEX: Mutex<Option<Index>> = Mutex::new(None);
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    Begin {
        source: String,
        root: PathBuf,
        #[serde(default)]
        directories: Vec<String>,
    },
    Baseline {
        source: String,
        receiver: String,
    },
    Ignore {
        source: String,
        relative: String,
        revision: String,
    },
    RetryIgnored {
        source: String,
    },
    States {
        source: String,
        receiver: String,
        relatives: Vec<String>,
    },
    Defer {
        source: String,
        relative: String,
        revision: String,
    },
    Pending {
        source: String,
        receiver: String,
    },
    IncludeExisting {
        source: String,
    },
    Step,
    Cancel,
    Summary {
        source: String,
    },
    Candidates {
        source: String,
        receiver: String,
    },
    Page {
        source: String,
        offset: usize,
        receiver: String,
    },
    Entry {
        source: String,
        relative: String,
    },
    Forget {
        source: String,
    },
    Prepare {
        source: String,
        name: String,
        root: PathBuf,
        relative: String,
        source_id: String,
        revision: String,
        receiver: String,
        metadata: BTreeMap<String, String>,
        paired: Option<(String, String)>,
    },
}
pub fn call(command: Command) -> Result<Value> {
    let host = sender()?;
    let mut guard = INDEX.lock().map_err(lock)?;
    if guard.is_none() {
        *guard = Some(Index::open(
            &host
                .request_root
                .parent()
                .ok_or(Error::NotFound)?
                .join("folders.sqlite3"),
        )?);
    }
    let index = guard.as_mut().unwrap();
    match command {
        Command::IncludeExisting { source } => {
            index.include_existing(&source)?;
            Ok(json!({}))
        }
        Command::Defer {
            source,
            relative,
            revision,
        } => {
            index.defer(
                &source,
                &relative,
                &revision,
                photobridge_folder_source::millis(SystemTime::now()) + 60_000,
            )?;
            Ok(json!({}))
        }
        Command::Pending { source, receiver } => {
            Ok(json!({"count":index.pending(&source,&receiver)?}))
        }
        Command::States {
            source,
            receiver,
            relatives,
        } => {
            if relatives.len() > 400 {
                return Err(Error::Invalid("folder visible page".into()));
            }
            let mut states = BTreeMap::new();
            let queue = host.sender.lock().map_err(lock)?;
            for relative in relatives {
                if let Ok(Some(id)) = index.job_id(&source, &relative, &receiver) {
                    let state = if id == 0 {
                        "excluded".to_owned()
                    } else {
                        serde_json::to_value(queue.job(id)?.state)?
                            .as_str()
                            .unwrap_or("queued")
                            .to_owned()
                    };
                    states.insert(relative, state);
                }
            }
            Ok(json!(states))
        }
        Command::Baseline { source, receiver } => {
            index.baseline(&source, &receiver)?;
            Ok(json!({}))
        }
        Command::Ignore {
            source,
            relative,
            revision,
        } => {
            index.ignore(&source, &relative, &revision)?;
            Ok(json!({}))
        }
        Command::RetryIgnored { source } => {
            index.retry_ignored(&source)?;
            Ok(json!({}))
        }
        Command::Begin {
            source,
            root,
            directories,
        } => {
            index.begin_scoped(
                &source,
                &root,
                photobridge_folder_source::millis(SystemTime::now()),
                &directories,
            )?;
            Ok(json!({}))
        }
        Command::Step => Ok(serde_json::to_value(
            index.step(photobridge_folder_source::millis(SystemTime::now()), 500)?,
        )?),
        Command::Cancel => {
            index.cancel();
            Ok(json!({}))
        }
        Command::Summary { source } => Ok(serde_json::to_value(index.summary(&source, 0)?)?),
        Command::Candidates { source, receiver } => Ok(serde_json::to_value(index.candidates(
            &source,
            &receiver,
            photobridge_folder_source::millis(SystemTime::now()),
            8,
        )?)?),
        Command::Page {
            source,
            offset,
            receiver,
        } => {
            let entries = index.page(&source, offset, &receiver)?;
            let mut rows = Vec::new();
            for e in entries {
                let mut row = serde_json::to_value(&e)?;
                if let Some(id) = e.job_id {
                    if let Ok(job) = host.sender.lock().map_err(lock)?.job(id) {
                        row["state"] = serde_json::to_value(job.state)?;
                    }
                }
                rows.push(row);
            }
            Ok(json!(rows))
        }
        Command::Entry { source, relative } => {
            Ok(serde_json::to_value(index.entry(&source, &relative)?)?)
        }
        Command::Forget { source } => {
            index.forget(&source)?;
            Ok(json!({}))
        }
        Command::Prepare {
            source,
            name,
            root,
            relative,
            source_id,
            revision: expected,
            receiver,
            mut metadata,
            paired,
        } => {
            // Identity/revision must come from this index, never a caller-forged asset ID.
            let candidate = index.entry(&source, &relative)?;
            if candidate.source_id != source_id || candidate.revision != expected {
                return Err(Error::Conflict("folder candidate changed".into()));
            }
            let mut originals = vec![(relative.clone(), expected.clone())];
            if let Some(pair) = paired {
                originals.push(pair);
            }
            for (p, r) in &originals {
                let entry = index.entry(&source, p)?;
                if entry.revision != *r
                    || !index.stable(
                        &source,
                        p,
                        photobridge_folder_source::millis(SystemTime::now()),
                    )?
                {
                    return Err(Error::Conflict("folder source settling".into()));
                }
            }
            let paths: Vec<_> = originals
                .iter()
                .map(|(p, _)| resolve(&root, p))
                .collect::<Result<_>>()?;
            for (i, path) in paths.iter().enumerate() {
                if revision(path)? != originals[i].1
                    || photobridge_folder_source::current_source_id(&source, path, &originals[i].0)?
                        != index.entry(&source, &originals[i].0)?.source_id
                {
                    return Err(Error::Conflict("folder source replaced".into()));
                }
            }
            let bytes = paths.iter().try_fold(0u64, |n, p| {
                Ok::<_, Error>(n.saturating_add(fs::metadata(p)?.len()))
            })?;
            let allowance = host.storage_status()?["export_allowance"]
                .as_u64()
                .unwrap_or(0);
            if bytes > allowance {
                return Err(Error::Capacity);
            }
            let combined = if originals.len() == 2 {
                format!("{}|{}", expected, originals[1].1)
            } else {
                expected.clone()
            };
            let states = host
                .sender
                .lock()
                .map_err(lock)?
                .source_states(&receiver, &[(source_id.clone(), combined.clone())])?;
            let job_id = if !states.contains_key(&source_id) {
                let mut random = [0u8; 16];
                getrandom::fill(&mut random).map_err(|_| Error::Storage("random source".into()))?;
                let folder = host.export_root.join(digest(&random));
                fs::create_dir_all(&folder)?;
                let result = (|| {
                    let mut resources = Vec::new();
                    for (i, path) in paths.iter().enumerate() {
                        if revision(path)? != originals[i].1 {
                            return Err(Error::Conflict("folder source changed".into()));
                        }
                        let filename = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .ok_or(Error::Invalid("folder filename".into()))?
                            .to_owned();
                        let output = folder.join(format!("{i}-{filename}"));
                        fs::copy(path, &output)?;
                        if revision(path)? != originals[i].1
                            || photobridge_folder_source::current_source_id(
                                &source,
                                path,
                                &originals[i].0,
                            )? != index.entry(&source, &originals[i].0)?.source_id
                            || fs::metadata(&output)?.len() != fs::metadata(path)?.len()
                        {
                            return Err(Error::Conflict(
                                "folder source changed during copy".into(),
                            ));
                        }
                        resources.push(ExportedResource {
                            role: if i > 0 {
                                ResourceRole::PairedVideo
                            } else if candidate.media_type.starts_with("video/") {
                                ResourceRole::Video
                            } else {
                                ResourceRole::Photo
                            },
                            filename,
                            media_type: photobridge_folder_source::media_type(path)
                                .ok_or(Error::Unsupported("folder media".into()))?
                                .into(),
                            path: output,
                        });
                    }
                    metadata.insert("source_type".into(), "folder".into());
                    metadata.insert("source_name".into(), name);
                    metadata.insert("source_ref".into(), source.clone());
                    let kind = if originals.len() == 2 {
                        AssetKind::Motion
                    } else if candidate.media_type.starts_with("video/") {
                        AssetKind::Video
                    } else {
                        AssetKind::Photo
                    };
                    let (asset, files) = manifest(source_id, combined, kind, metadata, resources)?;
                    let job = host.enqueue(&receiver, asset, files)?;
                    if let Ok(maintenance) = host.maintenance.lock() {
                        let _ = maintenance.log("transfer_queued", Some(job.id), None);
                    }
                    Ok::<_, Error>(job.id)
                })();
                match result {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = fs::remove_dir_all(folder);
                        return Err(e);
                    }
                }
            } else {
                host.sender
                    .lock()
                    .map_err(lock)?
                    .source_job_id(&receiver, &source_id, &combined)?
                    .ok_or(Error::NotFound)?
            };
            for (p, r) in originals {
                index.mark(&source, &p, &receiver, &r, job_id)?;
            }
            Ok(json!({}))
        }
    }
}
