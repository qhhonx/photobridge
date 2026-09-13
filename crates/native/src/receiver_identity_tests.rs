use super::*;
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "photobridge-relocation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn address() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}
#[tokio::test]
async fn relocated_receiver_keeps_identity_partial_bytes_and_receipts() {
    let root = Scratch::new();
    // Seed a legacy certificate for a different IP without requiring an OS alias.
    let legacy = identity(&root.0, "127.0.0.2:8484".parse().unwrap())
        .unwrap()
        .pairing;
    let before = fs::read(root.0.join("identity.json")).unwrap();
    let receiver = ReceiverHost::start(&root.0, address(), 10000)
        .await
        .unwrap();
    let saved = receiver.pairing.clone();
    assert_eq!(saved.receiver_id, legacy.receiver_id);
    assert!(saved.certificate == legacy.certificate && saved.token == legacy.token);
    assert_eq!(saved.certificate_name.as_deref(), Some("127.0.0.2"));
    let client = saved.client().unwrap();
    client.capabilities().await.unwrap();
    let bytes = b"synthetic original retained across receiver address changes";
    let asset = Asset {
        version: PROTOCOL_VERSION,
        source_id: "relocation-test".into(),
        revision: "1".into(),
        kind: AssetKind::Photo,
        metadata: BTreeMap::new(),
        resources: vec![Resource {
            role: ResourceRole::Photo,
            filename: "fixture.jpg".into(),
            media_type: "image/jpeg".into(),
            size: bytes.len() as u64,
            sha256: digest(bytes),
        }],
    };
    let state = client.register(&asset).await.unwrap();
    client
        .append(
            &state.asset_id,
            &asset.resources[0].sha256,
            0,
            bytes[..10].to_vec(),
        )
        .await
        .unwrap();
    drop(client);
    drop(receiver);
    // Allow the listener task to drop its Arc/store lock before reopening.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let receiver = ReceiverHost::start(&root.0, address(), 10000)
        .await
        .unwrap();
    let updated = relocated_pairing(&saved, &receiver.pairing.endpoint).unwrap();
    let client = updated.client().unwrap();
    let status = client.status(&state.asset_id).await.unwrap();
    assert_eq!(status.resources[0].offset, 10);
    client
        .append(
            &state.asset_id,
            &asset.resources[0].sha256,
            10,
            bytes[10..].to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(
        client.commit(&state.asset_id).await.unwrap().receipt,
        ReceiptState::Received
    );
    assert_eq!(
        client.register(&asset).await.unwrap().receipt,
        ReceiptState::Received
    );
    assert!(
        fs::read(root.0.join("identity.json")).unwrap() == before,
        "network relocation modified trust identity"
    );
    let mut wrong_name = updated.clone();
    wrong_name.certificate_name = Some("unrelated.invalid".into());
    assert!(wrong_name.client().unwrap().capabilities().await.is_err());
    let other_root = Scratch::new();
    let other = ReceiverHost::start(&other_root.0, address(), 10000)
        .await
        .unwrap();
    let impostor_route = relocated_pairing(&saved, &other.pairing.endpoint).unwrap();
    assert!(impostor_route
        .client()
        .unwrap()
        .capabilities()
        .await
        .is_err());
    let mut wrong_token = updated;
    wrong_token.token = digest(b"wrong bearer");
    assert!(matches!(
        wrong_token.client().unwrap().capabilities().await,
        Err(Error::Unauthorized)
    ));
}
#[test]
fn discovery_changes_only_a_valid_local_route() {
    let root = Scratch::new();
    let saved = identity(&root.0, "127.0.0.1:8484".parse().unwrap())
        .unwrap()
        .pairing;
    let moved = relocated_pairing(&saved, "https://192.168.1.22:8484").unwrap();
    assert!(saved.token == moved.token && saved.certificate == moved.certificate);
    assert_eq!(moved.receiver_id, saved.receiver_id);
    assert_eq!(moved.certificate_name.as_deref(), Some("127.0.0.1"));
    assert_eq!(
        relocated_pairing(&moved, "https://10.0.0.3:8484")
            .unwrap()
            .certificate_name,
        moved.certificate_name
    );
    for bad in [
        "http://192.168.1.1",
        "https://8.8.8.8",
        "https://example.com",
        "https://u@192.168.1.1",
        "https://192.168.1.1/path",
        "https://192.168.1.1?x=y",
    ] {
        assert!(relocated_pairing(&saved, bad).is_err());
    }
    let json = serde_json::to_value(&saved).unwrap();
    assert!(json.get("certificate_name").is_none());
    let old: Pairing = serde_json::from_value(json).unwrap();
    assert!(old.certificate_name.is_none());
    assert_eq!(
        receiver_connection_info(&root.0).unwrap(),
        json!({"endpoint":null})
    );
}
