#![cfg(feature = "folder-source")]
use photobridge_folder_source::{millis, Index};
use photobridge_native::{photobridge_call, photobridge_free};
use serde_json::{json, Value};
use std::{
    ffi::{CStr, CString},
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
fn call(v: Value) -> Value {
    let input = CString::new(v.to_string()).unwrap();
    unsafe {
        let p = photobridge_call(input.as_ptr());
        assert!(!p.is_null());
        let result: Value = serde_json::from_str(CStr::from_ptr(p).to_str().unwrap()).unwrap();
        photobridge_free(p);
        result
    }
}
fn ok(v: Value) -> Value {
    let r = call(v);
    assert_eq!(r["ok"], true, "{r}");
    r["value"].clone()
}
fn folder(v: Value) -> Value {
    ok(json!({"op":"folder","command":v}))
}
#[test]
fn folder_motion_uses_existing_tls_queue_and_only_reclaims_owned_snapshots() {
    let temp = std::env::temp_dir().join(format!(
        "pb-folder-flow-{}-{}",
        std::process::id(),
        millis(SystemTime::now())
    ));
    let source = temp.join("external");
    let queue = temp.join("app/queue");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&queue).unwrap();
    for (name, bytes) in [
        ("original.jpg", b"original image".as_slice()),
        ("original.mov", b"original video".as_slice()),
    ] {
        let path = source.join(name);
        fs::write(&path, bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1600000000))
            .unwrap();
    }
    let now = millis(SystemTime::now());
    let mut index = Index::open(&queue.join("folders.sqlite3")).unwrap();
    index.begin("source", &source, now - 20000).unwrap();
    while index.step(now - 20000, 100).unwrap().scanning {}
    let photo = index.entry("source", "original.jpg").unwrap();
    let video = index.entry("source", "original.mov").unwrap();
    drop(index);
    ok(json!({"op":"open_sender","root":queue}));
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let pairing = ok(
        json!({"op":"start_receiver","root":temp.join("receiver"),"listen":address.to_string(),"capacity":1000000}),
    );
    let receiver = pairing["receiver_id"].as_str().unwrap();
    let validation = folder(json!({"action":"validate_rules","include":["["],"exclude":[]}));
    assert!(validation["error"].as_str().unwrap().contains('['));
    assert_eq!(
        folder(json!({"action":"validate_rules","include":["**/*.jpg"],"exclude":["ignored/**"]}))
            ["error"],
        Value::Null
    );
    let prepare = json!({"action":"prepare","source":"source","name":"Test drive","root":source,"relative":photo.relative,"source_id":photo.source_id,"revision":photo.revision,"receiver":receiver,"metadata":{"created_at_ms":"1600000000000"},"paired":[video.relative,video.revision]});
    // A dismissed component must not be smuggled into a motion asset.
    folder(
        json!({"action":"dismiss","source":"source","relative":video.relative,"revision":video.revision}),
    );
    let rejected = call(json!({"op":"folder","command":prepare.clone()}));
    assert_eq!(rejected["ok"], false);
    assert!(ok(json!({"op":"jobs","receiver_id":receiver}))
        .as_array()
        .unwrap()
        .is_empty());
    // Reset only this synthetic source's inventory/dismissal, retaining originals.
    folder(json!({"action":"forget","source":"source"}));
    let mut index = Index::open(&queue.join("folders.sqlite3")).unwrap();
    index.begin("source", &source, now - 20000).unwrap();
    while index.step(now - 20000, 100).unwrap().scanning {}
    drop(index);
    folder(prepare.clone());
    folder(prepare.clone());
    let jobs = ok(json!({"op":"jobs","receiver_id":receiver}));
    assert_eq!(jobs.as_array().unwrap().len(), 1);
    assert_eq!(
        ok(json!({"op":"jobs","receiver_id":receiver,"source_filter":"library"}))
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        ok(json!({"op":"jobs","receiver_id":receiver,"source_filter":"source"}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(jobs[0]["asset"]["kind"], "motion");
    assert_eq!(jobs[0]["asset"]["resources"].as_array().unwrap().len(), 2);
    assert_eq!(
        ok(json!({"op":"sender_summary","receiver_id":receiver,"source_filter":"source"}))["total"],
        1
    );
    assert_eq!(
        ok(json!({"op":"sender_summary","receiver_id":receiver,"source_filter":"library"}))
            ["total"],
        0
    );
    ok(json!({"op":"pause_sender","paused":false}));
    let result = ok(json!({"op":"run_sender","pairing":pairing}));
    assert_eq!(result["state"], "received");
    folder(prepare);
    let page = folder(json!({"action":"page","source":"source","receiver":receiver,"offset":0}));
    assert!(page
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["state"] == "received"));
    assert_eq!(
        fs::read(source.join("original.jpg")).unwrap(),
        b"original image"
    );
    assert_eq!(
        fs::read(source.join("original.mov")).unwrap(),
        b"original video"
    );
    let exports = temp.join("app/exports");
    if exports.exists() {
        assert!(fs::read_dir(exports)
            .unwrap()
            .all(|e| fs::read_dir(e.unwrap().path()).unwrap().next().is_none()));
    }
    // Repeating the UI's explicit backup action must preserve received receipts.
    for _ in 0..2 {
        folder(json!({"action":"include_existing","source":"source"}));
        folder(json!({"action":"retry_ignored","source":"source"}));
        assert!(
            folder(json!({"action":"candidates","source":"source","receiver":receiver}))
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let sorted = folder(
        json!({"action":"page","source":"source","receiver":receiver,"offset":0,"sort":"path_desc"}),
    );
    assert_eq!(sorted[0]["relative"], "original.mov");
    assert_eq!(sorted[1]["relative"], "original.jpg");
    assert!(sorted
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["state"] == "received"));
    let reverse_children = folder(
        json!({"action":"children","source":"source","directory":"","receiver":receiver,"offset":0,"descending":true}),
    );
    assert_eq!(reverse_children["rows"][0]["relative"], "original.mov");
    let invalid_sort = call(
        json!({"op":"folder","command":{"action":"page","source":"source","receiver":receiver,"offset":0,"sort":"bad_sort"}}),
    );
    assert_eq!(invalid_sort["ok"], false);
    let children = folder(
        json!({"action":"children","source":"source","directory":"","receiver":receiver,"offset":0}),
    );
    assert_eq!(children["has_more"], false);
    assert_eq!(children["rows"].as_array().unwrap().len(), 2);
    assert!(children["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["entry"]["state"] == "received"));
    let preview = folder(
        json!({"action":"preview_entry","source":"source","job":jobs[0]["id"],"source_id":photo.source_id}),
    );
    assert_eq!(preview["relative"], photo.relative);
    assert_eq!(preview["source_id"], photo.source_id);
    assert!(folder(json!({"action":"preview_entry","source":"other-source","job":jobs[0]["id"],"source_id":photo.source_id})).is_null());
    // Changing a source after its indexed revision cannot queue a stale snapshot.
    fs::write(source.join("original.jpg"), b"changed image").unwrap();
    let response = call(
        json!({"op":"folder","command":{"action":"prepare","source":"source","name":"Test drive","root":source,"relative":photo.relative,"source_id":photo.source_id,"revision":photo.revision,"receiver":"other","metadata":{},"paired":null}}),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"], "conflict");
    let _ = fs::remove_dir_all(temp);
}
