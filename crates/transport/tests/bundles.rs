use photobridge_core::*;
use photobridge_store::Receiver;
use photobridge_transport::{router, Client};
use std::{
    collections::BTreeMap,
    io::Cursor,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQ: AtomicU64 = AtomicU64::new(0);
const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "photobridge-bundle-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn sample(motion: bool) -> (Asset, Vec<Vec<u8>>, Vec<u8>) {
    sample_sized(motion, MAX_CHUNK_BYTES + 117)
}
fn sample_sized(motion: bool, length: usize) -> (Asset, Vec<Vec<u8>>, Vec<u8>) {
    let mut originals = vec![vec![7; length]];
    if motion {
        originals.push(vec![9; 17]);
    }
    let resources = originals
        .iter()
        .enumerate()
        .map(|(i, bytes)| Resource {
            role: if i == 0 {
                ResourceRole::Photo
            } else {
                ResourceRole::PairedVideo
            },
            filename: format!("fixture-{i}"),
            media_type: "application/octet-stream".into(),
            size: bytes.len() as u64,
            sha256: digest(bytes),
        })
        .collect();
    let asset = Asset {
        version: 1,
        source_id: "bundle-fixture".into(),
        revision: "1".into(),
        kind: if motion {
            AssetKind::Motion
        } else {
            AssetKind::Photo
        },
        metadata: BTreeMap::new(),
        resources,
    };
    let mut bytes = Vec::new();
    let size = write_bundle(&asset, &mut bytes, |r| {
        Ok(Box::new(Cursor::new(
            originals
                .iter()
                .find(|v| digest(v) == r.sha256)
                .unwrap()
                .clone(),
        )))
    })
    .unwrap();
    assert_eq!(size, bytes.len() as u64);
    (asset, originals, bytes)
}
async fn serve(root: &Fixture, capacity: u64) -> (String, tokio::task::JoinHandle<()>) {
    let receiver = Receiver::open(&root.0, capacity).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router(receiver, TOKEN).unwrap())
            .await
            .unwrap();
    });
    (base, task)
}
fn request(base: &str, bytes: Vec<u8>, chunked: bool) -> reqwest::RequestBuilder {
    let builder = reqwest::Client::new()
        .post(format!("{base}/v1/bundles"))
        .bearer_auth(TOKEN)
        .header("content-type", BUNDLE_CONTENT_TYPE);
    if chunked {
        let chunks: Vec<_> = bytes
            .chunks(64 * 1024)
            .map(|s| Ok::<_, std::io::Error>(s.to_vec()))
            .collect();
        builder.body(reqwest::Body::wrap_stream(futures_util::stream::iter(
            chunks,
        )))
    } else {
        builder.body(bytes)
    }
}
#[tokio::test]
async fn bundle_is_atomic_per_asset_and_resumes_non_aligned_legacy_prefix() {
    let root = Fixture::new();
    let (base, task) = serve(&root, 32 << 20).await;
    let (asset, originals, body) = sample(true);
    let client = Client::new(&base, TOKEN).unwrap();
    assert!(client.capabilities().await.unwrap().bundle_upload);
    let status = client.register(&asset).await.unwrap();
    client
        .append(
            &status.asset_id,
            &asset.resources[0].sha256,
            0,
            originals[0][..123].to_vec(),
        )
        .await
        .unwrap();
    let response = request(&base, body.clone(), false).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let receipt: AssetStatus = response.json().await.unwrap();
    assert_eq!(receipt.receipt, ReceiptState::Received);
    assert_eq!(receipt.resources.len(), 2);
    assert!(receipt.resources.iter().all(|r| r.complete));
    let replay = request(&base, body, false).send().await.unwrap();
    assert_eq!(replay.status(), 200);
    assert_eq!(
        replay.json::<AssetStatus>().await.unwrap().asset_id,
        receipt.asset_id
    );
    task.abort();
}
#[tokio::test]
async fn interrupted_bundle_keeps_verified_prefix_without_committing() {
    let root = Fixture::new();
    let (base, task) = serve(&root, 32 << 20).await;
    let (asset, _, body) = sample(false);
    let prefix = 12 + serde_json::to_vec(&asset).unwrap().len() + MAX_CHUNK_BYTES;
    let response = request(&base, body[..prefix].to_vec(), true)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let client = Client::new(&base, TOKEN).unwrap();
    let partial = client.status(&asset.id().unwrap()).await.unwrap();
    assert_eq!(partial.receipt, ReceiptState::Receiving);
    assert_eq!(partial.resources[0].offset, MAX_CHUNK_BYTES as u64);
    assert_eq!(
        request(&base, body, false).send().await.unwrap().status(),
        200
    );
    task.abort();
}
#[tokio::test]
async fn malformed_oversized_corrupt_and_unauthorized_bundles_never_get_receipts() {
    let root = Fixture::new();
    let (base, task) = serve(&root, 32 << 20).await;
    // Keep rejection probes small: a server may reset an HTTP/1 connection
    // before a large unauthorized body finishes writing. Streaming coverage above
    // uses multi-megabyte resources and asserts durable offsets and full receipts.
    let (asset, originals, body) = sample_sized(false, 17);
    let mut invalid = body.clone();
    invalid[0] = 0;
    assert_eq!(
        request(&base, invalid, false)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let mut huge = BUNDLE_MAGIC.to_vec();
    huge.extend_from_slice(&(MAX_MANIFEST_BYTES as u32 + 1).to_be_bytes());
    assert_eq!(
        request(&base, huge, true).send().await.unwrap().status(),
        400
    );
    let unauthorized = reqwest::Client::new()
        .post(format!("{base}/v1/bundles"))
        .header("content-type", BUNDLE_CONTENT_TYPE)
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    let mut corrupt = body.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert_eq!(
        request(&base, corrupt, false)
            .send()
            .await
            .unwrap()
            .status(),
        422
    );
    assert_ne!(
        Client::new(&base, TOKEN)
            .unwrap()
            .status(&asset.id().unwrap())
            .await
            .unwrap()
            .receipt,
        ReceiptState::Received
    );
    let mut staging = Vec::new();
    assert!(
        write_bundle(&asset, &mut staging, |_| Ok(Box::new(Cursor::new(
            vec![0; originals[0].len()]
        ))))
        .is_err()
    );
    task.abort();
    let small = Fixture::new();
    let (base, task) = serve(&small, 1).await;
    assert_eq!(
        request(&base, body, false).send().await.unwrap().status(),
        507
    );
    task.abort();
}
#[test]
fn old_capability_payload_does_not_opt_into_bundle_transport() {
    let old: Capabilities = serde_json::from_str(
        r#"{"version":1,"max_chunk_bytes":4194304,"motion_assets":true,"target_processing":[]}"#,
    )
    .unwrap();
    assert!(!old.bundle_upload);
}
