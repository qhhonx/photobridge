use photobridge_core::*;
use photobridge_native::{ReceiverHost, SenderHost};
use photobridge_sender::JobState;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQ: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "photobridge-native-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn address() -> std::net::SocketAddr {
    let s = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    s.local_addr().unwrap()
}
#[tokio::test]
async fn native_queue_transfers_over_paired_tls_and_preserves_originals() {
    let t = Temp::new();
    let r = ReceiverHost::start(&t.0.join("receiver"), address(), 100000)
        .await
        .unwrap();
    let sender = SenderHost::open(&t.0.join("sender")).unwrap();
    let photo = b"original photo bytes";
    let video = b"original paired video bytes";
    let mut paths = BTreeMap::new();
    let mut resources = Vec::new();
    for (bytes, name, role, mime) in [
        (
            photo.as_slice(),
            "photo.heic",
            ResourceRole::Photo,
            "image/heic",
        ),
        (
            video.as_slice(),
            "video.mov",
            ResourceRole::PairedVideo,
            "video/quicktime",
        ),
    ] {
        let p = t.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        let sha256 = digest(bytes);
        paths.insert(sha256.clone(), p.to_str().unwrap().to_string());
        resources.push(Resource {
            role,
            filename: name.into(),
            media_type: mime.into(),
            size: bytes.len() as u64,
            sha256,
        });
    }
    let a = Asset {
        version: 1,
        source_id: "ios-resource".into(),
        revision: "1".into(),
        kind: AssetKind::Motion,
        metadata: BTreeMap::from([("created_at_ms".into(), "1780000000000".into())]),
        resources,
    };
    sender
        .enqueue(&r.pairing.receiver_id, a.clone(), paths.clone())
        .unwrap();
    sender.pause(true).unwrap();
    assert!(sender.run_once(&r.pairing).await.unwrap().is_none());
    sender.pause(false).unwrap();
    let job = sender.run_once(&r.pairing).await.unwrap().unwrap();
    assert_eq!(job.state, JobState::Received);
    assert_eq!(job.confirmed_bytes, (photo.len() + video.len()) as u64);
    {
        let mut receiver = r.receiver.lock().unwrap();
        assert_eq!(
            receiver.asset(&a.id().unwrap()).unwrap().metadata,
            a.metadata
        );
        let overview = receiver.overview().unwrap();
        assert_eq!(overview["received"], 1);
        assert_eq!(overview["published"], 0);
        assert_eq!(
            overview["recent"][0]["confirmed_bytes"],
            (photo.len() + video.len()) as u64
        );
        assert!(!overview.to_string().contains("sources"));
        let publications = receiver.publications("", 50).unwrap();
        assert_eq!(publications.len(), 1);
        for resource in &a.resources {
            assert_eq!(
                digest_reader(
                    std::fs::File::open(&publications[0].resources[&resource.sha256]).unwrap()
                )
                .unwrap(),
                resource.sha256
            );
        }
        receiver
            .set_processing(&a.id().unwrap(), ProcessingState::Pending)
            .unwrap();
        receiver
            .set_processing(&a.id().unwrap(), ProcessingState::Failed)
            .unwrap();
    }
    for p in paths.values() {
        std::fs::remove_file(p).unwrap();
    }
    drop(sender);
    let sender = SenderHost::open(&t.0.join("sender")).unwrap();
    assert_eq!(sender.list(0).unwrap()[0].state, JobState::Received);
    assert!(sender.run_once(&r.pairing).await.unwrap().is_none());
}
#[tokio::test]
async fn tls_rejects_a_different_receiver_certificate_and_bad_credentials() {
    let t = Temp::new();
    let a = ReceiverHost::start(&t.0.join("a"), address(), 10000)
        .await
        .unwrap();
    let b = ReceiverHost::start(&t.0.join("b"), address(), 10000)
        .await
        .unwrap();
    assert!(a.pairing.client().unwrap().capabilities().await.is_ok());
    let mut wrong = a.pairing.clone();
    wrong.certificate = b.pairing.certificate.clone();
    wrong.receiver_id = b.pairing.receiver_id.clone();
    assert!(wrong.client().unwrap().capabilities().await.is_err());
    let mut bad = a.pairing.clone();
    bad.token = digest(b"wrong-token");
    assert!(matches!(
        bad.client().unwrap().capabilities().await,
        Err(Error::Unauthorized)
    ));
    let mut changed = a.pairing.clone();
    changed.receiver_id = digest(b"wrong-identity");
    assert!(changed.client().is_err());
}

/// Execute exactly the platform-independent requests handed to URLSession.
/// Lost responses and restarts must query the receiver, never infer receipt.
#[tokio::test]
async fn external_executor_recovers_lost_chunk_and_commit_receipts() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use photobridge_native::NativeRequest;
    use std::collections::BTreeSet;
    let t = Temp::new();
    let r = ReceiverHost::start(&t.0.join("receiver"), address(), 16 * 1024 * 1024)
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .add_root_certificate(
            reqwest::Certificate::from_der(&STANDARD.decode(&r.pairing.certificate).unwrap())
                .unwrap(),
        )
        .tls_built_in_root_certs(false)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    async fn execute(
        client: &reqwest::Client,
        r: &ReceiverHost,
        request: &NativeRequest,
    ) -> (u16, String) {
        let response = client
            .request(
                request.method.parse().unwrap(),
                format!("{}{}", r.pairing.endpoint, request.path),
            )
            .bearer_auth(&r.pairing.token)
            .header("content-type", &request.content_type)
            .body(std::fs::read(&request.body_file).unwrap())
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }
    let bytes = vec![7u8; MAX_CHUNK_BYTES + 37];
    let video = vec![9u8; 31];
    let mut resources = Vec::new();
    let mut sources = BTreeMap::new();
    for (content, role, name, mime) in [
        (&bytes, ResourceRole::Photo, "photo.heic", "image/heic"),
        (
            &video,
            ResourceRole::PairedVideo,
            "video.mov",
            "video/quicktime",
        ),
    ] {
        let path = t.0.join(name);
        std::fs::write(&path, content).unwrap();
        let hash = digest(content);
        sources.insert(hash.clone(), path.to_str().unwrap().into());
        resources.push(Resource {
            role,
            filename: name.into(),
            media_type: mime.into(),
            size: content.len() as u64,
            sha256: hash,
        });
    }
    let asset = Asset {
        version: 1,
        source_id: "external-live".into(),
        revision: "1".into(),
        kind: AssetKind::Motion,
        metadata: BTreeMap::new(),
        resources,
    };
    let root = t.0.join("sender");
    let mut sender = SenderHost::open(&root).unwrap();
    let job = sender
        .enqueue(&r.pairing.receiver_id, asset.clone(), sources.clone())
        .unwrap();
    let mut lost_chunk = false;
    let mut lost_commit = false;
    for task in 1..20 {
        let Some(request) = sender.prepare_native(&r.pairing.receiver_id).unwrap() else {
            break;
        };
        let task_id = task.to_string();
        sender.bind_native(&request.attempt, &task_id).unwrap();
        assert!(sender
            .prepare_native(&r.pairing.receiver_id)
            .unwrap()
            .is_none());
        let (code, body) = execute(&client, &r, &request).await;
        assert_eq!(code, 200);
        if (!lost_chunk && request.method == "PUT")
            || (!lost_commit && request.path.ends_with("/commit"))
        {
            if request.method == "PUT" {
                lost_chunk = true;
            } else {
                lost_commit = true;
            }
            // OS finished but the process died before its callback was persisted.
            drop(sender);
            sender = SenderHost::open(&root).unwrap();
            assert!(sender
                .prepare_native(&r.pairing.receiver_id)
                .unwrap()
                .is_none());
            sender.reconcile_native(&BTreeSet::new()).unwrap();
            assert!(sender
                .finish_native(&request.attempt, &task_id, code, &body, None, false)
                .is_err());
        } else {
            sender
                .finish_native(&request.attempt, &task_id, code, &body, None, false)
                .unwrap();
        }
    }
    assert!(lost_chunk && lost_commit);
    let jobs = sender.list(0).unwrap();
    assert_eq!(jobs[0].state, JobState::Received);
    assert_eq!(jobs[0].confirmed_bytes, (bytes.len() + video.len()) as u64);
    assert_eq!(
        sender
            .enqueue(&r.pairing.receiver_id, asset.clone(), sources)
            .unwrap()
            .id,
        job.id
    );
    let publications = r.receiver.lock().unwrap().publications("", 50).unwrap();
    assert_eq!(publications.len(), 1);
    for resource in &asset.resources {
        assert_eq!(
            digest_reader(
                std::fs::File::open(&publications[0].resources[&resource.sha256]).unwrap()
            )
            .unwrap(),
            resource.sha256
        );
    }
    assert_eq!(std::fs::read_dir(root.join("requests")).unwrap().count(), 0);
}

#[test]
fn external_executor_classifies_retries_and_rejects_bad_acknowledgements() {
    use photobridge_sender::Failure;
    let t = Temp::new();
    let s = SenderHost::open(&t.0.join("sender")).unwrap();
    let path = t.0.join("photo.jpg");
    std::fs::write(&path, b"photo").unwrap();
    let hash = digest(b"photo");
    let a = Asset {
        version: 1,
        source_id: "retry-test".into(),
        revision: "1".into(),
        kind: AssetKind::Photo,
        metadata: BTreeMap::new(),
        resources: vec![Resource {
            role: ResourceRole::Photo,
            filename: "photo.jpg".into(),
            media_type: "image/jpeg".into(),
            size: 5,
            sha256: hash.clone(),
        }],
    };
    let id = s
        .enqueue(
            "receiver",
            a,
            BTreeMap::from([(hash, path.to_str().unwrap().into())]),
        )
        .unwrap()
        .id;
    for (code, expected) in [
        (503, JobState::Waiting),
        (507, JobState::Waiting),
        (401, JobState::Failed),
        (422, JobState::Failed),
        (200, JobState::Failed),
    ] {
        let request = s.prepare_native("receiver").unwrap().unwrap();
        s.bind_native(&request.attempt, "task").unwrap();
        let job = s
            .finish_native(&request.attempt, "task", code, "{}", None, false)
            .unwrap();
        assert_eq!(job.state, expected);
        assert_eq!(job.confirmed_bytes, 0);
        assert!(s.prepare_native("receiver").unwrap().is_none());
        s.retry(id).unwrap();
    }
    let request = s.prepare_native("receiver").unwrap().unwrap();
    s.bind_native(&request.attempt, "cancelled").unwrap();
    s.pause(true).unwrap();
    assert_eq!(
        s.finish_native(
            &request.attempt,
            "cancelled",
            0,
            "",
            Some(Failure::Network),
            true
        )
        .unwrap()
        .state,
        JobState::Queued
    );
    assert!(s.prepare_native("receiver").unwrap().is_none());
    s.pause(false).unwrap();
    let next = s.prepare_native("receiver").unwrap().unwrap();
    assert_eq!(next.path, "/v1/assets");
    assert!(s
        .finish_native(&request.attempt, "cancelled", 200, "{}", None, false)
        .is_err());
}

/// Run explicitly for sustained queue/transport validation without user media.
#[tokio::test]
#[ignore = "300-item bounded stress fixture"]
async fn bulk_mixed_queue_survives_pause_restart_and_replay() {
    let t = Temp::new();
    let addr = address();
    let root = t.0.join("receiver");
    let mut receiver = ReceiverHost::start(&root, addr, 256 << 20).await.unwrap();
    let mut sender = SenderHost::open(&t.0.join("sender")).unwrap();
    let mut resources = Vec::new();
    let mut paths = BTreeMap::new();
    for (name, role, mime, size) in [
        ("fixture.jpg", ResourceRole::Photo, "image/jpeg", 200_000),
        (
            "fixture.mov",
            ResourceRole::PairedVideo,
            "video/quicktime",
            400_000,
        ),
    ] {
        let bytes = vec![if name.ends_with("jpg") { 7 } else { 9 }; size];
        let hash = digest(&bytes);
        let path = t.0.join(name);
        std::fs::write(&path, &bytes).unwrap();
        paths.insert(hash.clone(), path.to_str().unwrap().to_owned());
        resources.push(Resource {
            role,
            filename: name.into(),
            media_type: mime.into(),
            size: size as u64,
            sha256: hash,
        });
    }
    for index in 0..300 {
        let (kind, mut selected) = match index % 3 {
            0 => (AssetKind::Photo, vec![resources[0].clone()]),
            1 => {
                let mut video = resources[1].clone();
                video.role = ResourceRole::Video;
                (AssetKind::Video, vec![video])
            }
            _ => (AssetKind::Motion, resources.clone()),
        };
        let mut sources = BTreeMap::new();
        for resource in &mut selected {
            let mut bytes = std::fs::read(&paths[&resource.sha256]).unwrap();
            bytes[..8].copy_from_slice(&(index as u64).to_le_bytes());
            resource.sha256 = digest(&bytes);
            let path = t.0.join(format!("{index}-{}", resource.filename));
            std::fs::write(&path, bytes).unwrap();
            let path = path.to_str().unwrap().to_owned();
            paths.insert(resource.sha256.clone(), path.clone());
            sources.insert(resource.sha256.clone(), path);
        }
        sender
            .enqueue(
                &receiver.pairing.receiver_id,
                Asset {
                    version: 1,
                    source_id: format!("bulk-{index}"),
                    revision: "1".into(),
                    kind,
                    resources: selected,
                    metadata: BTreeMap::new(),
                },
                sources,
            )
            .unwrap();
    }
    let start = std::time::Instant::now();
    for index in 0..300 {
        if index == 120 {
            sender.pause(true).unwrap();
            assert!(sender.run_once(&receiver.pairing).await.unwrap().is_none());
            drop(sender);
            drop(receiver);
            // Aborted server tasks release their shared store on the next runtime turn.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            receiver = loop {
                match ReceiverHost::start(&root, addr, 256 << 20).await {
                    Ok(host) => break host,
                    Err(Error::Conflict(_)) if std::time::Instant::now() < deadline => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await
                    }
                    Err(error) => panic!("receiver restart failed: {error}"),
                }
            };
            sender = SenderHost::open(&t.0.join("sender")).unwrap();
            assert!(sender.run_once(&receiver.pairing).await.unwrap().is_none());
            sender.pause(false).unwrap();
        }
        let job = sender.run_once(&receiver.pairing).await.unwrap().unwrap();
        assert_eq!(job.state, JobState::Received);
        assert_eq!(
            job.confirmed_bytes,
            job.asset.resources.iter().map(|r| r.size).sum::<u64>()
        );
    }
    assert!(sender.run_once(&receiver.pairing).await.unwrap().is_none());
    let first = sender.list(0).unwrap();
    let second = sender.list(first.last().unwrap().id).unwrap();
    assert_eq!(first.len() + second.len(), 300);
    for path in paths.values() {
        std::fs::remove_file(path).unwrap();
    }
    drop(sender);
    let sender = SenderHost::open(&t.0.join("sender")).unwrap();
    assert!(sender.run_once(&receiver.pairing).await.unwrap().is_none());
    let overview = receiver.receiver.lock().unwrap().overview().unwrap();
    assert_eq!(overview["received"], 300);
    assert_eq!(overview["reserved_bytes"], 120000000);
    eprintln!("BULK_RESULT items=300 photos=100 videos=100 motion=100 elapsed_ms={} stored_bytes=120000000 pause_restart_at=120 replay_without_sources=passed",start.elapsed().as_millis());
}

#[tokio::test]
async fn local_receiver_settings_work_before_network_and_survive_first_start() {
    let t = Temp::new();
    let root = t.0.join("offline-receiver");
    let request = |value: serde_json::Value| -> serde_json::Value {
        let output: serde_json::Value =
            serde_json::from_str(&photobridge_native::call(&value.to_string())).unwrap();
        assert_eq!(output["ok"], true, "{output}");
        output["value"].clone()
    };
    let initial = request(serde_json::json!({"op":"receiver_settings","root":root}));
    let mut settings = initial["settings"].clone();
    settings["receiver_budget_bytes"] = serde_json::json!(13u64 << 30);
    settings["log_days"] = serde_json::json!(30);
    request(serde_json::json!({"op":"receiver_settings","root":root,"settings":settings}));
    let history = request(
        serde_json::json!({"op":"receiver_history","root":root,"state":"all","kind":"all"}),
    );
    assert_eq!(history["total"], 0);
    assert_eq!(history["items"], serde_json::json!([]));
    assert!(
        !request(serde_json::json!({"op":"receiver_logs","root":root}))
            .as_array()
            .unwrap()
            .is_empty()
    );
    let host = ReceiverHost::start(&root, address(), 10000).await.unwrap();
    assert_eq!(
        host.receiver.lock().unwrap().overview().unwrap()["capacity_bytes"],
        13u64 << 30
    );
}

#[test]
fn offline_archive_preserves_originals_until_all_resources_verify() {
    let t = Temp::new();
    let root = t.0.join("offline-archive");
    let call = |value: serde_json::Value| -> serde_json::Value {
        serde_json::from_str(&photobridge_native::call(&value.to_string())).unwrap()
    };
    let batch = || call(serde_json::json!({"op":"archive_batch","root":root}));
    assert_eq!(batch()["ok"], false);
    assert!(
        !root.exists(),
        "Archive inspection must not create an empty receiver"
    );
    let mut receiver = photobridge_store::Receiver::open(root.join("store"), 1 << 20).unwrap();
    let originals = [
        b"original still".as_slice(),
        b"original motion video".as_slice(),
    ];
    let asset = Asset {
        version: 1,
        source_id: "offline-motion".into(),
        revision: "1".into(),
        kind: AssetKind::Motion,
        metadata: BTreeMap::new(),
        resources: originals
            .iter()
            .enumerate()
            .map(|(i, bytes)| Resource {
                role: if i == 0 {
                    ResourceRole::Photo
                } else {
                    ResourceRole::PairedVideo
                },
                filename: if i == 0 { "still.heic" } else { "motion.mov" }.into(),
                media_type: if i == 0 {
                    "image/heic"
                } else {
                    "video/quicktime"
                }
                .into(),
                size: bytes.len() as u64,
                sha256: digest(bytes),
            })
            .collect(),
    };
    let id = asset.id().unwrap();
    receiver.register(asset.clone()).unwrap();
    for (resource, bytes) in asset.resources.iter().zip(originals) {
        receiver
            .append(&id, &resource.sha256, 0, bytes, &resource.sha256)
            .unwrap();
    }
    receiver.commit(&id).unwrap();
    assert_eq!(
        batch()["error"],
        "conflict",
        "A separate writer must not be bypassed"
    );
    drop(receiver);
    assert_eq!(
        batch()["value"],
        serde_json::json!([]),
        "Unpublished items cannot be archived"
    );
    let mut receiver = photobridge_store::Receiver::open(root.join("store"), 1 << 20).unwrap();
    receiver
        .set_processing(&id, ProcessingState::Pending)
        .unwrap();
    receiver
        .set_processing(&id, ProcessingState::Complete)
        .unwrap();
    drop(receiver);
    let before = batch();
    assert_eq!(before["ok"], true);
    assert_eq!(before["value"].as_array().unwrap().len(), 1);
    assert_eq!(
        before["value"][0]["asset"]["resources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let release = |verified: Vec<String>| {
        call(serde_json::json!({
            "op":"release_archived","root":root,"ids":[id],"verified":verified
        }))
    };
    assert_eq!(
        release(vec![asset.resources[0].sha256.clone()])["error"],
        "integrity"
    );
    for (resource, bytes) in asset.resources.iter().zip(originals) {
        let path = before["value"][0]["resources"][&resource.sha256]
            .as_str()
            .unwrap();
        assert_eq!(
            std::fs::read(path).unwrap(),
            bytes,
            "Rejected release must retain every original"
        );
    }
    assert_eq!(
        call(
            serde_json::json!({"op":"record_event","receiver":true,"root":root,
        "code":"original_archive_verified"})
        )["ok"],
        true
    );
    let released = release(asset.resources.iter().map(|r| r.sha256.clone()).collect());
    assert_eq!(released["ok"], true);
    assert_eq!(
        released["value"]["bytes"],
        originals.iter().map(|b| b.len() as u64).sum::<u64>()
    );
    assert_eq!(batch()["value"], serde_json::json!([]));
    let mut receiver = photobridge_store::Receiver::open(root.join("store"), 1 << 20).unwrap();
    assert_eq!(
        receiver.status(&id).unwrap().receipt,
        ReceiptState::Received
    );
    assert!(receiver.originals_released(&id).unwrap());
    assert_eq!(
        receiver.register(asset).unwrap().receipt,
        ReceiptState::Received
    );
    assert!(
        !root.join("identity.json").exists(),
        "Offline archive must not create a network identity"
    );
    let logs = call(serde_json::json!({"op":"receiver_logs","root":root}));
    let codes: Vec<_> = logs["value"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["code"].as_str())
        .collect();
    assert!(codes.contains(&"original_archive_verified"));
    assert!(codes.contains(&"originals_reclaimed"));
    assert!(!codes.contains(&"receiver_started"));
}
