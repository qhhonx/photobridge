use photobridge_core::*;
use photobridge_native::ReceiverHost;
use photobridge_store::devices::DeviceDirectory;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQ: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "photobridge-device-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn names_survive_language_changes_and_concurrent_open() {
    let temp = Temp::new();
    let original = DeviceDirectory::open(&temp.0, "zh-Hans")
        .unwrap()
        .profile()
        .unwrap();
    assert!(!original.name.is_ascii());
    let workers: Vec<_> = (0..6)
        .map(|_| {
            let root = temp.0.clone();
            std::thread::spawn(move || {
                DeviceDirectory::open(&root, "en")
                    .unwrap()
                    .profile()
                    .unwrap()
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), original);
    }
    let directory = DeviceDirectory::open(&temp.0, "en").unwrap();
    let changed = directory.rename("  Living Room 🦊  ").unwrap();
    assert_eq!(changed.id, original.id);
    assert_eq!(changed.name, "Living Room 🦊");
    for name in ["", "\n", "name\nnext", "hidden\u{202e}", &"a".repeat(41)] {
        assert!(directory.rename(name).is_err());
        assert_eq!(directory.profile().unwrap(), changed);
    }
    assert_eq!(
        DeviceDirectory::open(&temp.0, "zh")
            .unwrap()
            .profile()
            .unwrap(),
        changed
    );
}

#[tokio::test]
async fn authenticated_profile_exchange_keeps_multiple_peers_and_media_identity() {
    let temp = Temp::new();
    let root = temp.0.join("receiver");
    let receiver_devices = DeviceDirectory::open(&root.join("store"), "zh").unwrap();
    receiver_devices.rename("琥珀水獭").unwrap();
    let bind = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = bind.local_addr().unwrap();
    drop(bind);
    let receiver = ReceiverHost::start(&root, address, 1 << 20).await.unwrap();
    let first = DeviceDirectory::open(&temp.0.join("first"), "en").unwrap();
    let second = DeviceDirectory::open(&temp.0.join("second"), "en").unwrap();
    let one = first.rename("Moonlit Cedar").unwrap();
    let two = second.rename("Moonlit Cedar").unwrap();
    assert_ne!(one.id, two.id);
    let client = receiver.pairing.client().unwrap();
    let remote = client.exchange_device_profile(&one).await.unwrap().unwrap();
    assert_eq!(remote.name, "琥珀水獭");
    first
        .remember(&receiver.pairing.receiver_id, &remote)
        .unwrap();
    client.exchange_device_profile(&two).await.unwrap();
    let mut wrong = receiver.pairing.clone();
    wrong.token = digest(b"wrong token");
    assert!(wrong
        .client()
        .unwrap()
        .exchange_device_profile(&DeviceProfile {
            id: digest(b"attacker"),
            name: "Not accepted".into()
        })
        .await
        .is_err());
    assert_eq!(receiver_devices.peers().unwrap().len(), 2);
    let bytes = b"unchanged original";
    let asset = Asset {
        version: 1,
        source_id: "fixture".into(),
        revision: "1".into(),
        kind: AssetKind::Photo,
        metadata: BTreeMap::new(),
        resources: vec![Resource {
            role: ResourceRole::Photo,
            filename: "photo.jpg".into(),
            media_type: "image/jpeg".into(),
            size: bytes.len() as u64,
            sha256: digest(bytes),
        }],
    };
    let id = asset.id().unwrap();
    client.register(&asset).await.unwrap();
    client
        .append(&id, &digest(bytes), 0, bytes.to_vec())
        .await
        .unwrap();
    client.commit(&id).await.unwrap();
    let updated = first.rename("Kitchen Mac").unwrap();
    client.exchange_device_profile(&updated).await.unwrap();
    receiver_devices.rename("Sunny Otter").unwrap();
    let remote = client
        .exchange_device_profile(&updated)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(remote.name, "Sunny Otter");
    first
        .remember(&receiver.pairing.receiver_id, &remote)
        .unwrap();
    assert_eq!(first.peers().unwrap().len(), 1);
    assert_eq!(receiver_devices.peers().unwrap().len(), 2);
    assert!(receiver_devices
        .peers()
        .unwrap()
        .iter()
        .any(|p| p.profile.id == one.id && p.profile.name == "Kitchen Mac"));
    assert_eq!(
        client.register(&asset).await.unwrap().receipt,
        ReceiptState::Received
    );
    assert_eq!(
        receiver.receiver.lock().unwrap().overview().unwrap()["total"],
        1
    );
    assert_eq!(
        DeviceDirectory::open(&temp.0.join("first"), "zh")
            .unwrap()
            .peers()
            .unwrap()[0]
            .profile
            .name,
        "Sunny Otter"
    );
    // Pairing schema remains unchanged: no display fields are inserted into QR credentials.
    let credential = serde_json::to_value(&receiver.pairing).unwrap();
    assert_eq!(credential.as_object().unwrap().len(), 5);
}

#[tokio::test]
async fn older_receiver_without_profile_endpoint_is_compatible() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, axum::Router::new()).await.unwrap();
    });
    let client = photobridge_transport::Client::new(
        &format!("http://{address}"),
        &digest(b"test credential"),
    )
    .unwrap();
    let profile = DeviceProfile {
        id: digest(b"installation"),
        name: "Amber Otter".into(),
    };
    assert!(client
        .exchange_device_profile(&profile)
        .await
        .unwrap()
        .is_none());
    server.abort();
}
