use photobridge_core::*;
use photobridge_pixel::PixelTarget;
use photobridge_store::Receiver;
use photobridge_transport::{router, Client};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "photobridge-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn asset(motion: bool) -> (Asset, Vec<Vec<u8>>) {
    let mut bytes = vec![b"original photograph with metadata".to_vec()];
    if motion {
        bytes.push(b"original paired video with metadata".to_vec());
    }
    let resources = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| Resource {
            role: if i == 0 {
                ResourceRole::Photo
            } else {
                ResourceRole::PairedVideo
            },
            filename: if i == 0 { "photo.jpg" } else { "paired.mov" }.into(),
            media_type: if i == 0 {
                "image/jpeg"
            } else {
                "video/quicktime"
            }
            .into(),
            size: b.len() as u64,
            sha256: digest(b),
        })
        .collect();
    (
        Asset {
            version: PROTOCOL_VERSION,
            source_id: "fixture-asset".into(),
            revision: "1".into(),
            kind: if motion {
                AssetKind::Motion
            } else {
                AssetKind::Photo
            },
            metadata: BTreeMap::from([("capture_time".into(), "2026-01-01T00:00:00Z".into())]),
            resources,
        },
        bytes,
    )
}
fn receive(receiver: &mut Receiver, asset: &Asset, bytes: &[Vec<u8>]) -> AssetStatus {
    let state = receiver.register(asset.clone()).unwrap();
    for (r, b) in asset.resources.iter().zip(bytes) {
        receiver
            .append(&state.asset_id, &r.sha256, 0, b, &digest(b))
            .unwrap();
    }
    receiver.commit(&state.asset_id).unwrap()
}
#[test]
fn validates_asset_shapes_paths_versions_and_stable_ids() {
    let (a, _) = asset(true);
    let id = a.id().unwrap();
    let decoded: Asset = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
    assert_eq!(id, decoded.id().unwrap());
    let mut changed = a.clone();
    changed.resources.pop();
    assert!(changed.validate().is_err());
    for name in [
        "../photo.jpg",
        "folder/photo.jpg",
        "folder\\photo.jpg",
        "..",
        "photo\n.jpg",
    ] {
        let mut changed = a.clone();
        changed.resources[0].filename = name.into();
        assert!(changed.validate().is_err());
    }
    let mut changed = a.clone();
    changed.version = 2;
    assert!(matches!(changed.validate(), Err(Error::Unsupported(_))));
    let mut changed = a.clone();
    changed.revision = "2".into();
    assert_ne!(id, changed.id().unwrap());
}
#[test]
fn resource_resume_survives_receiver_restart_and_lost_response() {
    let root = Scratch::new();
    let (a, bytes) = asset(false);
    let id = a.id().unwrap();
    let r = &a.resources[0];
    {
        let mut store = Receiver::open(&root.0, 10000).unwrap();
        store.register(a.clone()).unwrap();
        store
            .append(&id, &r.sha256, 0, &bytes[0][..7], &digest(&bytes[0][..7]))
            .unwrap();
        assert!(Receiver::open(&root.0, 10000).is_err());
    }
    let mut store = Receiver::open(&root.0, 10000).unwrap();
    assert_eq!(store.status(&id).unwrap().resources[0].offset, 7);
    // Retry an accepted chunk whose response was lost.
    assert_eq!(
        store
            .append(&id, &r.sha256, 0, &bytes[0][..7], &digest(&bytes[0][..7]))
            .unwrap()
            .resources[0]
            .offset,
        7
    );
    assert!(matches!(
        store.append(&id, &r.sha256, 0, b"wrong!!", &digest(b"wrong!!")),
        Err(Error::Conflict(_))
    ));
    store
        .append(&id, &r.sha256, 7, &bytes[0][7..], &digest(&bytes[0][7..]))
        .unwrap();
    assert_eq!(store.commit(&id).unwrap().receipt, ReceiptState::Received);
    assert_eq!(store.commit(&id).unwrap().receipt, ReceiptState::Received);
    assert_eq!(store.register(a).unwrap().receipt, ReceiptState::Received);
}
#[test]
fn motion_commit_is_atomic_and_processing_never_reuploads() {
    let root = Scratch::new();
    let (a, bytes) = asset(true);
    let mut store = Receiver::open(&root.0, 10000).unwrap();
    let id = store.register(a.clone()).unwrap().asset_id;
    store
        .append(
            &id,
            &a.resources[0].sha256,
            0,
            &bytes[0],
            &digest(&bytes[0]),
        )
        .unwrap();
    assert!(matches!(store.commit(&id), Err(Error::Conflict(_))));
    assert!(store.set_processing(&id, ProcessingState::Pending).is_err());
    assert_eq!(store.status(&id).unwrap().receipt, ReceiptState::Receiving);
    store
        .append(
            &id,
            &a.resources[1].sha256,
            0,
            &bytes[1],
            &digest(&bytes[1]),
        )
        .unwrap();
    store.commit(&id).unwrap();
    store.set_processing(&id, ProcessingState::Pending).unwrap();
    store.set_processing(&id, ProcessingState::Failed).unwrap();
    let status = store.register(a.clone()).unwrap();
    assert!(matches!(
        next_action(&a, &status, MAX_CHUNK_BYTES).unwrap(),
        TransferAction::Done
    ));
    assert_eq!(status.processing, ProcessingState::Failed);
    store.set_processing(&id, ProcessingState::Pending).unwrap();
    store
        .set_processing(&id, ProcessingState::Complete)
        .unwrap();
    assert!(store.set_processing(&id, ProcessingState::Pending).is_err());
}
#[test]
fn integrity_rejects_wrong_chunks_and_wrong_whole_file() {
    let root = Scratch::new();
    let (a, bytes) = asset(false);
    let mut store = Receiver::open(&root.0, 10000).unwrap();
    let id = store.register(a.clone()).unwrap().asset_id;
    let hash = &a.resources[0].sha256;
    assert!(matches!(
        store.append(&id, hash, 0, &bytes[0], &digest(b"wrong")),
        Err(Error::Integrity)
    ));
    assert_eq!(store.status(&id).unwrap().resources[0].offset, 0);
    let wrong = vec![b'x'; bytes[0].len()];
    assert!(matches!(
        store.append(&id, hash, 0, &wrong, &digest(&wrong)),
        Err(Error::Integrity)
    ));
    assert_eq!(store.status(&id).unwrap().resources[0].offset, 0);
    receive(&mut store, &a, &bytes);
    fs::write(root.0.join("blobs").join(hash), wrong).unwrap();
    assert!(matches!(store.commit(&id), Err(Error::Integrity)));
}
#[test]
fn capacity_reservations_are_atomic_and_deduplicate_content() {
    let root = Scratch::new();
    let (a, bytes) = asset(true);
    let mut store = Receiver::open(&root.0, bytes[0].len() as u64).unwrap();
    assert!(matches!(store.register(a), Err(Error::Capacity)));
    let (photo, bytes) = asset(false);
    receive(&mut store, &photo, &bytes);
    let mut second = photo.clone();
    second.source_id = "another-device".into();
    let status = store.register(second).unwrap();
    assert!(status.resources[0].complete);
    let mut different = photo;
    different.resources[0].sha256 = digest(b"different");
    assert!(matches!(store.register(different), Err(Error::Capacity)));
}
#[test]
fn restart_recovers_full_partial_and_post_rename_crashes() {
    for renamed in [false, true] {
        let root = Scratch::new();
        let (a, bytes) = asset(false);
        let id = a.id().unwrap();
        let hash = a.resources[0].sha256.clone();
        {
            let mut store = Receiver::open(&root.0, 10000).unwrap();
            store.register(a).unwrap();
        }
        // Emulate durable file bytes before SQLite's ready flag was committed.
        fs::write(
            root.0
                .join(if renamed { "blobs" } else { "partial" })
                .join(hash),
            &bytes[0],
        )
        .unwrap();
        let mut store = Receiver::open(&root.0, 10000).unwrap();
        assert!(store.status(&id).unwrap().resources[0].complete);
        store.commit(&id).unwrap();
    }
}
#[test]
fn pixel_output_is_optional_and_does_not_claim_an_unimplemented_converter() {
    let (motion, _) = asset(true);
    let (photo, _) = asset(false);
    assert_eq!(
        PixelTarget::default().plan(&photo).unwrap(),
        TargetPlan::PublishOriginals
    );
    assert!(matches!(
        PixelTarget::default().plan(&motion),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(
        PixelTarget {
            motion_conversion_available: true
        }
        .plan(&motion)
        .unwrap(),
        TargetPlan::GenerateMotionPhoto
    );
}
async fn start(root: &Scratch) -> (String, tokio::task::JoinHandle<()>) {
    let app = router(
        Receiver::open(root.0.join("receiver"), 100_000_000).unwrap(),
        TOKEN,
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task)
}
#[tokio::test(flavor = "multi_thread")]
async fn http_end_to_end_authentication_resume_and_repeat() {
    let root = Scratch::new();
    let (url, server) = start(&root).await;
    let client = Client::new(&url, TOKEN).unwrap();
    assert_eq!(
        client.capabilities().await.unwrap().version,
        PROTOCOL_VERSION
    );
    assert!(client
        .capabilities()
        .await
        .unwrap()
        .target_processing
        .is_empty());
    let denied = reqwest::get(format!("{url}/v1/capabilities"))
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    let (a, bytes) = asset(true);
    let id = a.id().unwrap();
    let mut paths = BTreeMap::new();
    for (r, b) in a.resources.iter().zip(&bytes) {
        let path = root.0.join(&r.filename);
        fs::write(&path, b).unwrap();
        paths.insert(r.sha256.clone(), path);
    }
    client.register(&a).await.unwrap();
    client
        .append(&id, &a.resources[0].sha256, 0, bytes[0][..8].to_vec())
        .await
        .unwrap();
    drop(client); // A newly created sender has no in-memory transfer state.
    let client = Client::new(&url, TOKEN).unwrap();
    let mut first_offset = None;
    let result = client
        .send(&a, &paths, &AtomicBool::new(false), |s| {
            first_offset.get_or_insert(s.resources[0].offset);
        })
        .await
        .unwrap();
    assert_eq!(first_offset, Some(8));
    assert_eq!(result.receipt, ReceiptState::Received);
    // A received asset needs neither the original paths nor another upload.
    let result = client
        .send(&a, &BTreeMap::new(), &AtomicBool::new(false), |_| {})
        .await
        .unwrap();
    assert_eq!(result.receipt, ReceiptState::Received);
    for (r, b) in a.resources.iter().zip(&bytes) {
        assert_eq!(
            fs::read(root.0.join("receiver/blobs").join(&r.sha256)).unwrap(),
            *b
        );
    }
    server.abort();
    let _ = server.await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cancelled_and_oversized_requests_never_commit() {
    let root = Scratch::new();
    let (url, server) = start(&root).await;
    let client = Client::new(&url, TOKEN).unwrap();
    let (a, _) = asset(false);
    let id = a.id().unwrap();
    assert!(matches!(
        client
            .send(&a, &BTreeMap::new(), &AtomicBool::new(true), |_| {})
            .await,
        Err(Error::Cancelled)
    ));
    assert_eq!(
        client.status(&id).await.unwrap().receipt,
        ReceiptState::Receiving
    );
    let response = reqwest::Client::new()
        .put(format!(
            "{url}/v1/assets/{id}/resources/{}?offset=0&sha256={}",
            a.resources[0].sha256,
            digest(b"")
        ))
        .bearer_auth(TOKEN)
        .body(vec![0; MAX_CHUNK_BYTES + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert_eq!(client.status(&id).await.unwrap().resources[0].offset, 0);
    assert!(Client::new("http://192.168.1.3:8484", TOKEN).is_err());
    server.abort();
    let _ = server.await;
}

#[test]
fn storage_reserve_blocks_new_bytes_but_never_loses_receipts() {
    let t = Scratch::new();
    let mut receiver = Receiver::open(&t.0, 100000).unwrap();
    let (a, bytes) = asset(true);
    receiver.configure_storage(100000, 1 << 60).unwrap();
    assert!(matches!(receiver.register(a.clone()), Err(Error::LowSpace)));
    assert_eq!(receiver.overview().unwrap()["reserved_bytes"], 0);
    receiver.configure_storage(1, 0).unwrap();
    assert!(matches!(receiver.register(a.clone()), Err(Error::Capacity)));
    receiver.configure_storage(100000, 0).unwrap();
    let registered = receiver.register(a.clone()).unwrap();
    receiver.configure_storage(100000, 1 << 60).unwrap();
    let r = &a.resources[0];
    assert!(matches!(
        receiver.append(
            &registered.asset_id,
            &r.sha256,
            0,
            &bytes[0],
            &digest(&bytes[0])
        ),
        Err(Error::LowSpace)
    ));
    assert_eq!(
        receiver.status(&registered.asset_id).unwrap().resources[0].offset,
        0
    );
    receiver.configure_storage(100000, 0).unwrap();
    receive(&mut receiver, &a, &bytes);
    receiver.configure_storage(1, 1 << 60).unwrap();
    assert_eq!(
        receiver.register(a.clone()).unwrap().receipt,
        ReceiptState::Received
    );
    assert_eq!(
        receiver.commit(&a.id().unwrap()).unwrap().receipt,
        ReceiptState::Received
    );
}

#[test]
fn archive_reclamation_requires_all_originals_and_preserves_shared_refs_and_dedupe() {
    let t = Scratch::new();
    let mut receiver = Receiver::open(&t.0, 100000).unwrap();
    let (first, bytes) = asset(true);
    let mut second = first.clone();
    second.source_id = "second".into();
    receive(&mut receiver, &first, &bytes);
    receive(&mut receiver, &second, &bytes);
    let ids = vec![first.id().unwrap()];
    let proof = first.resources.iter().map(|r| r.sha256.clone()).collect();
    assert!(receiver.release_archived(&ids, &proof).is_err()); // Unpublished originals are retained.
    for a in [&first, &second] {
        receiver
            .set_processing(&a.id().unwrap(), ProcessingState::Pending)
            .unwrap();
        receiver
            .set_processing(&a.id().unwrap(), ProcessingState::Complete)
            .unwrap();
    }
    assert_eq!(receiver.archive_batch().unwrap().len(), 2);
    assert!(receiver
        .release_archived(&ids, &std::collections::BTreeSet::new())
        .is_err());
    assert_eq!(receiver.release_archived(&ids, &proof).unwrap(), 0); // Another asset needs the same blobs.
    assert_eq!(receiver.archive_batch().unwrap().len(), 1);
    let freed = receiver
        .release_archived(&[second.id().unwrap()], &proof)
        .unwrap();
    assert_eq!(freed, first.resources.iter().map(|r| r.size).sum::<u64>());
    drop(receiver);
    let mut receiver = Receiver::open(&t.0, 100000).unwrap();
    assert_eq!(
        receiver.register(first.clone()).unwrap().receipt,
        ReceiptState::Received
    );
    assert_eq!(
        receiver.commit(&first.id().unwrap()).unwrap().receipt,
        ReceiptState::Received
    );
    assert_eq!(receiver.release_archived(&ids, &proof).unwrap(), 0);
    assert_eq!(receiver.overview().unwrap()["reserved_bytes"], 0);
    let mut third = first.clone();
    third.source_id = "third".into();
    receive(&mut receiver, &third, &bytes);
    assert_eq!(
        receiver.status(&first.id().unwrap()).unwrap().receipt,
        ReceiptState::Received
    );
}

#[test]
#[cfg(unix)]
fn archived_reclamation_recovers_after_file_removal_is_interrupted() {
    let t = Scratch::new();
    let mut receiver = Receiver::open(&t.0, 100000).unwrap();
    let (asset, bytes) = asset(false);
    receive(&mut receiver, &asset, &bytes);
    let id = asset.id().unwrap();
    receiver
        .set_processing(&id, ProcessingState::Pending)
        .unwrap();
    receiver
        .set_processing(&id, ProcessingState::Complete)
        .unwrap();
    let proof = asset.resources.iter().map(|r| r.sha256.clone()).collect();
    let blob = t.0.join("blobs").join(&asset.resources[0].sha256);
    use std::os::unix::fs::PermissionsExt;
    let directory = t.0.join("blobs");
    // Simulate an I/O failure after the durable release marker is committed.
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = receiver.release_archived(std::slice::from_ref(&id), &proof);
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert_eq!(
        receiver.status(&id).unwrap().receipt,
        ReceiptState::Received
    );
    assert!(blob.exists());
    drop(receiver);
    let receiver = Receiver::open(&t.0, 100000).unwrap();
    assert!(!blob.exists());
    assert_eq!(receiver.overview().unwrap()["reserved_bytes"], 0);
    assert_eq!(
        receiver.status(&id).unwrap().receipt,
        ReceiptState::Received
    );
}

#[test]
fn receiver_catalog_filters_before_pagination_and_survives_stop() {
    use photobridge_store::catalog::Catalog;
    let root = Scratch::new();
    let mut receiver = Receiver::open(&root.0, 1 << 20).unwrap();
    let mut failed_id = String::new();
    for index in 0..205 {
        let (mut item, bytes) = asset(index % 2 == 0);
        item.source_id = format!("catalog-{index}");
        receiver.register(item.clone()).unwrap();
        if index == 0 {
            failed_id = receive(&mut receiver, &item, &bytes).asset_id;
            receiver
                .set_processing(&failed_id, ProcessingState::Pending)
                .unwrap();
            receiver
                .set_processing(&failed_id, ProcessingState::Failed)
                .unwrap();
        }
    }
    let catalog = Catalog::open(&root.0).unwrap();
    let first = catalog.page(None, "all", "all", 100).unwrap();
    assert_eq!(first.total, 205);
    assert_eq!(first.items.len(), 100);
    let second = catalog.page(first.next_cursor, "all", "all", 100).unwrap();
    let last = catalog.page(second.next_cursor, "all", "all", 100).unwrap();
    assert_eq!(last.items.len(), 5);
    assert!(last.next_cursor.is_none());
    let ids: std::collections::BTreeSet<_> = first
        .items
        .iter()
        .chain(&second.items)
        .chain(&last.items)
        .map(|r| &r.id)
        .collect();
    assert_eq!(ids.len(), 205);
    let failed = catalog.page(None, "failed", "motion", 100).unwrap();
    assert_eq!(failed.total, 1);
    assert_eq!(failed.items[0].id, failed_id);
    assert_eq!(failed.items[0].confirmed_bytes, failed.items[0].total_bytes);
    assert_eq!(
        catalog.page(None, "receiving", "photo", 100).unwrap().total,
        102
    );
    assert!(catalog.page(None, "invalid", "all", 100).is_err());
    drop(catalog);
    drop(receiver);
    assert_eq!(
        Catalog::open(&root.0)
            .unwrap()
            .page(None, "all", "all", 100)
            .unwrap()
            .total,
        205
    );
}

#[test]
fn receiver_catalog_never_repairs_or_removes_partial_files() {
    use photobridge_store::catalog::Catalog;
    let root = Scratch::new();
    let mut receiver = Receiver::open(&root.0, 1 << 20).unwrap();
    let (item, _) = asset(false);
    receiver.register(item.clone()).unwrap();
    let resource = &item.resources[0];
    let partial = root.0.join("partial").join(&resource.sha256);
    fs::write(&partial, vec![7; resource.size as usize]).unwrap();
    let page = Catalog::open(&root.0)
        .unwrap()
        .page(None, "all", "all", 100)
        .unwrap();
    assert_eq!(page.items[0].receipt, "receiving");
    assert!(partial.exists());
    assert!(!root.0.join("blobs").join(&resource.sha256).exists());
    let absent = root.0.join("not-created");
    assert!(Catalog::open(&absent)
        .unwrap()
        .page(None, "all", "all", 100)
        .unwrap()
        .items
        .is_empty());
    assert!(!absent.exists());
}

#[test]
fn relay_requires_opt_in_and_matching_delivery_evidence_and_keeps_receipts() {
    use photobridge_store::{catalog::Catalog, retention::GalleryCopy};
    let t = Scratch::new();
    let mut receiver = Receiver::open(&t.0, 100000).unwrap();
    let (first, bytes) = asset(true);
    let mut second = first.clone();
    second.source_id = "other-relay-source".into();
    let id = first.id().unwrap();
    let copy = GalleryCopy {
        locator: "content://media/external_primary/images/media/11".into(),
        sha256: digest(b"converted motion delivery"),
        size: 25,
    };
    receiver.register(first.clone()).unwrap();
    assert!(receiver.record_gallery_copy(&id, &copy).is_err());
    receive(&mut receiver, &first, &bytes);
    receive(&mut receiver, &second, &bytes);
    receiver.prepare_gallery_copy(&id, &copy).unwrap();
    drop(receiver);
    let mut receiver = Receiver::open(&t.0, 100000).unwrap();
    assert_eq!(
        receiver.expected_gallery_copy(&id).unwrap(),
        Some(copy.clone())
    );
    assert!(receiver.gallery_copy(&id).unwrap().is_none());
    assert!(receiver.release_gallery_copy(&id, &copy, true).is_err());
    receiver.record_gallery_copy(&id, &copy).unwrap();
    assert!(receiver.expected_gallery_copy(&id).unwrap().is_none());
    assert!(receiver.release_gallery_copy(&id, &copy, false).is_err());
    let mut changed = copy.clone();
    changed.sha256 = digest(b"tampered delivery");
    assert!(receiver.record_gallery_copy(&id, &changed).is_err());
    assert!(receiver.release_gallery_copy(&id, &changed, true).is_err());
    assert!(!receiver.originals_released(&id).unwrap());
    assert_eq!(receiver.release_gallery_copy(&id, &copy, true).unwrap(), 0); // Shared originals retained.
    let other = second.id().unwrap();
    receiver.record_gallery_copy(&other, &copy).unwrap();
    assert_eq!(
        receiver.release_gallery_copy(&other, &copy, true).unwrap(),
        bytes.iter().map(|b| b.len() as u64).sum::<u64>()
    );
    drop(receiver);
    let mut receiver = Receiver::open(&t.0, 100000).unwrap();
    assert_eq!(
        receiver.register(first).unwrap().receipt,
        ReceiptState::Received
    );
    assert_eq!(
        receiver.commit(&id).unwrap().receipt,
        ReceiptState::Received
    );
    assert_eq!(receiver.release_gallery_copy(&id, &copy, true).unwrap(), 0);
    let history = Catalog::open(&t.0)
        .unwrap()
        .page(None, "all", "all", 100)
        .unwrap();
    assert!(history
        .items
        .iter()
        .all(|i| i.originals_released && i.release_reason.as_deref() == Some("gallery")));
    // Original archives still demand original hashes, never delivery hashes.
    assert!(receiver
        .release_archived(&[id], &[copy.sha256].into_iter().collect())
        .is_err());
}

#[test]
fn maintenance_hold_preserves_offsets_and_receipts_until_resumed() {
    let root = Scratch::new();
    let mut receiver = Receiver::open(&root.0, 1 << 20).unwrap();
    let (asset, bytes) = asset(false);
    let id = asset.id().unwrap();
    receiver.register(asset.clone()).unwrap();
    let hash = &asset.resources[0].sha256;
    receiver
        .append(&id, hash, 0, &bytes[0][..3], &digest(&bytes[0][..3]))
        .unwrap();
    receiver.hold_transfers(true);
    assert!(matches!(
        receiver.register(asset.clone()),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        receiver.append(&id, hash, 3, &bytes[0][3..], &digest(&bytes[0][3..])),
        Err(Error::Conflict(_))
    ));
    assert_eq!(receiver.status(&id).unwrap().resources[0].offset, 3);
    receiver.hold_transfers(false);
    receiver
        .append(&id, hash, 3, &bytes[0][3..], &digest(&bytes[0][3..]))
        .unwrap();
    receiver.commit(&id).unwrap();
    receiver.hold_transfers(true);
    assert_eq!(
        receiver.register(asset).unwrap().receipt,
        ReceiptState::Received
    );
}

#[tokio::test]
async fn request_diagnostics_preserve_upload_bytes_and_ignore_private_headers() {
    use std::sync::{Arc, Mutex};
    let scratch = Scratch::new();
    let receiver = Arc::new(Mutex::new(Receiver::open(&scratch.0, 1024 * 1024).unwrap()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = events.clone();
    let app = photobridge_transport::shared_router_observed(
        receiver.clone(),
        TOKEN,
        Some(Arc::new(move |event| recorded.lock().unwrap().push(event))),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let host = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let (asset, resources) = asset(false);
    let manifest = serde_json::to_vec(&asset).unwrap();
    let response = client
        .post(format!("{url}/v1/assets"))
        .bearer_auth(TOKEN)
        .header("x-photobridge-request", "123")
        .body(manifest.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let status: AssetStatus = response.json().await.unwrap();
    let b = &resources[0];
    let hash = &asset.resources[0].sha256;
    let response = client
        .put(format!(
            "{url}/v1/assets/{}/resources/{hash}?offset=0&sha256={hash}",
            status.asset_id
        ))
        .bearer_auth(TOKEN)
        .header("x-photobridge-request", "124")
        .body(b.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = client
        .post(format!("{url}/v1/assets/{}/commit", status.asset_id))
        .bearer_auth(TOKEN)
        .header("x-photobridge-request", "private-name@example.com")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        receiver
            .lock()
            .unwrap()
            .status(&status.asset_id)
            .unwrap()
            .receipt,
        ReceiptState::Received
    );
    assert_eq!(fs::read(scratch.0.join("blobs").join(hash)).unwrap(), *b);
    let before = events.lock().unwrap().len();
    assert_eq!(
        client
            .post(format!("{url}/v1/assets"))
            .header("x-photobridge-request", "125")
            .body(manifest.clone())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let events = events.lock().unwrap();
    assert_eq!(events.len(), before);
    for (id, size) in [(123, manifest.len()), (124, b.len())] {
        let observed: Vec<_> = events.iter().filter(|e| e.request_id == id).collect();
        assert_eq!(observed.len(), 3);
        assert_eq!(observed[0].event, "receiver_request_started");
        assert_eq!(observed[1].event, "receiver_first_body");
        assert_eq!(observed[2].event, "receiver_response_ready");
        assert_eq!(observed[2].bytes_received, size as u64);
        assert!(observed[2].body_complete);
        assert_eq!(observed[2].status, Some(200));
    }
    assert_eq!(events.len(), 6);
    host.abort();
}
