use photobridge_core::*;
use photobridge_store::{catalog::Catalog, devices::DeviceDirectory, Receiver};
use std::{collections::BTreeMap, io::Cursor, net::SocketAddr};

#[tokio::test]
async fn sender_pause_preserves_receipts_and_filters_before_pagination() {
    let root = std::env::temp_dir().join(format!("photobridge-senders-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let token = "a".repeat(64);
    let a = DeviceProfile {
        id: digest(b"phone"),
        name: "Phone".into(),
    };
    let b = DeviceProfile {
        id: digest(b"mac"),
        name: "Mac".into(),
    };
    let receiver = Receiver::open(&root, 1 << 20).unwrap();
    let router = photobridge_transport::router(receiver, &token).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let http = reqwest::Client::new();
    for (profile, kind) in [(&a, "iPhone"), (&b, "Mac")] {
        let response = http
            .post(format!("{base}/v1/device-profile"))
            .bearer_auth(&token)
            .header("x-photobridge-sender", &profile.id)
            .header("x-photobridge-device-type", kind)
            .json(profile)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
    let directory = DeviceDirectory::open(&root, "en").unwrap();
    let peers = directory.peers().unwrap();
    assert_eq!(peers.len(), 2);
    assert!(peers.iter().all(|p| p.ip.as_deref() == Some("127.0.0.1")));
    assert_eq!(
        peers
            .iter()
            .find(|p| p.profile.id == a.id)
            .unwrap()
            .device_type
            .as_deref(),
        Some("iPhone")
    );
    let bytes = vec![7u8; 123];
    let asset = Asset {
        version: 1,
        source_id: "fixture".into(),
        revision: "1".into(),
        kind: AssetKind::Photo,
        metadata: BTreeMap::new(),
        resources: vec![Resource {
            role: ResourceRole::Photo,
            filename: "fixture.jpg".into(),
            media_type: "image/jpeg".into(),
            size: bytes.len() as u64,
            sha256: digest(&bytes),
        }],
    };
    let mut body = vec![];
    write_bundle(&asset, &mut body, |_| {
        Ok(Box::new(Cursor::new(bytes.clone())))
    })
    .unwrap();
    let send = |sender: &str| {
        http.post(format!("{base}/v1/bundles"))
            .bearer_auth(&token)
            .header("x-photobridge-sender", sender)
            .header("content-type", BUNDLE_CONTENT_TYPE)
            .body(body.clone())
    };
    directory.set_enabled(&a.id, false).unwrap();
    assert_eq!(send(&a.id).send().await.unwrap().status(), 409);
    assert_eq!(send(&b.id).send().await.unwrap().status(), 200);
    let catalog = Catalog::open(&root).unwrap();
    assert_eq!(
        catalog
            .page_for_sender(None, "all", "all", 1, Some(&a.id))
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        catalog
            .page_for_sender(None, "all", "all", 1, Some(&b.id))
            .unwrap()
            .total,
        1
    );
    // Re-enable and replay the same asset. One receipt, two proven senders.
    directory.set_enabled(&a.id, true).unwrap();
    assert_eq!(send(&a.id).send().await.unwrap().status(), 200);
    let page = catalog.page(None, "all", "all", 100).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].senders.len(), 2);
    assert_eq!(
        catalog
            .page_for_sender(None, "all", "all", 1, Some("unknown"))
            .unwrap()
            .total,
        0
    );
    directory.set_enabled(&a.id, false).unwrap();
    drop(directory);
    assert!(!DeviceDirectory::open(&root, "en")
        .unwrap()
        .enabled(&a.id)
        .unwrap());
    server.abort();
    let _ = server.await;
    drop(catalog);
    std::fs::remove_dir_all(root).unwrap();
}
