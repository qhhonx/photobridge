use photobridge_core::*;
use photobridge_sender::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQ: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "photobridge-queue-{}-{}",
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
fn asset() -> Asset {
    Asset {
        version: 1,
        source_id: "native:42".into(),
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
    }
}
fn add(s: &mut Sender) -> Job {
    let a = asset();
    let sources = BTreeMap::from([(
        a.resources[0].sha256.clone(),
        "opaque-native-resource-reference".into(),
    )]);
    s.enqueue("receiver-1", a, sources).unwrap()
}
fn status(j: &Job, received: bool) -> AssetStatus {
    AssetStatus {
        asset_id: j.asset.id().unwrap(),
        receipt: if received {
            ReceiptState::Received
        } else {
            ReceiptState::Receiving
        },
        processing: ProcessingState::NotRequested,
        resources: vec![ResourceStatus {
            sha256: j.asset.resources[0].sha256.clone(),
            offset: if received { 10 } else { 5 },
            complete: received,
        }],
    }
}
#[test]
fn restart_resumes_and_keeps_pause_progress_and_dedupe() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    let id = add(&mut s).id;
    assert_eq!(id, add(&mut s).id);
    let j = s.claim("receiver-1", 10).unwrap().unwrap();
    s.acknowledge(&j.attempt(), &status(&j, false)).unwrap();
    s.set_paused(true).unwrap();
    drop(s);
    let mut s = Sender::open(&t.0).unwrap();
    assert!(s.claim("receiver-1", 11).unwrap().is_none());
    assert_eq!(s.job(id).unwrap().confirmed_bytes, 5);
    s.set_paused(false).unwrap();
    let next = s.claim("receiver-1", 12).unwrap().unwrap();
    assert!(next.generation > j.generation);
    assert!(s.acknowledge(&j.attempt(), &status(&j, true)).is_err());
    s.acknowledge(&next.attempt(), &status(&next, true))
        .unwrap();
    s.retry(id).unwrap();
    assert_eq!(s.job(id).unwrap().state, JobState::Received);
    assert!(s.claim("receiver-1", 999).unwrap().is_none());
}
#[test]
fn transient_failure_is_automatic_permanent_failure_requires_action() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    let id = add(&mut s).id;
    for f in [
        Failure::Network,
        Failure::Capacity,
        Failure::LowSpace,
        Failure::Busy,
    ] {
        let j = s.claim("receiver-1", 10000).unwrap().unwrap();
        s.fail(&j.attempt(), f, 10000).unwrap();
        let waiting = s.job(id).unwrap();
        assert_eq!(waiting.state, JobState::Waiting);
        assert!(s
            .claim("receiver-1", waiting.next_attempt_at - 1)
            .unwrap()
            .is_none());
        let j = s
            .claim("receiver-1", waiting.next_attempt_at)
            .unwrap()
            .unwrap();
        s.interrupted(&j.attempt()).unwrap();
    }
    let j = s.claim("receiver-1", 20000).unwrap().unwrap();
    s.fail(&j.attempt(), Failure::Authentication, 20000)
        .unwrap();
    assert!(s.claim("receiver-1", i64::MAX).unwrap().is_none());
    s.retry(id).unwrap();
    assert!(s.claim("receiver-1", 20001).unwrap().is_some());
}
#[test]
fn native_task_survives_restart_and_lost_completion_queries_receiver() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    add(&mut s);
    let j = s.claim("receiver-1", 0).unwrap().unwrap();
    s.bind_native_task(&j.attempt(), "session:42").unwrap();
    drop(s);
    let mut s = Sender::open(&t.0).unwrap();
    assert!(s.claim("receiver-1", 0).unwrap().is_none());
    let cancel = s
        .reconcile_native(&BTreeSet::from([
            "session:42".into(),
            "session:stale".into(),
        ]))
        .unwrap();
    assert_eq!(cancel, vec!["session:stale"]);
    assert!(s.claim("receiver-1", 0).unwrap().is_none());
    s.reconcile_native(&BTreeSet::new()).unwrap();
    let next = s.claim("receiver-1", 0).unwrap().unwrap();
    assert!(s.acknowledge(&j.attempt(), &status(&j, true)).is_err());
    s.acknowledge(&next.attempt(), &status(&next, true))
        .unwrap();
}
#[test]
fn pause_invalidates_native_callbacks_and_multiple_destinations_are_independent() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    let first = add(&mut s);
    let second = s
        .enqueue("receiver-2", first.asset.clone(), first.sources.clone())
        .unwrap();
    assert_ne!(first.id, second.id);
    let j = s.claim("receiver-1", 0).unwrap().unwrap();
    s.bind_native_task(&j.attempt(), "session:task").unwrap();
    assert_eq!(s.pause_job(j.id).unwrap().as_deref(), Some("session:task"));
    assert!(s.acknowledge(&j.attempt(), &status(&j, true)).is_err());
    assert!(s.claim("receiver-1", 999).unwrap().is_none());
    assert!(s.claim("receiver-2", 0).unwrap().is_some());
}
#[test]
fn invalid_ack_cannot_complete_and_root_has_single_owner() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    assert!(Sender::open(&t.0).is_err());
    add(&mut s);
    let j = s.claim("receiver-1", 0).unwrap().unwrap();
    let mut st = status(&j, true);
    st.asset_id = digest(b"other");
    assert!(s.acknowledge(&j.attempt(), &st).is_err());
    assert_eq!(s.job(j.id).unwrap().state, JobState::Running);
}

#[test]
fn reexport_repairs_missing_sources_without_duplicating_the_asset() {
    let t = Temp::new();
    let mut sender = Sender::open(&t.0).unwrap();
    let first = add(&mut sender);
    let running = sender.claim("receiver-1", 0).unwrap().unwrap();
    sender
        .fail(&running.attempt(), Failure::SourceUnavailable, 0)
        .unwrap();
    let replacement = BTreeMap::from([(
        first.asset.resources[0].sha256.clone(),
        "new-export-reference".into(),
    )]);
    let repaired = sender
        .enqueue("receiver-1", first.asset, replacement.clone())
        .unwrap();
    assert_eq!(repaired.id, first.id);
    assert_eq!(repaired.sources, replacement);
    assert_eq!(repaired.state, JobState::Queued);
    assert_eq!(sender.list(0, 500).unwrap().len(), 1);
    assert!(sender
        .acknowledge(&running.attempt(), &status(&running, true))
        .is_err());
}

#[test]
fn native_ack_is_atomic_and_only_its_bound_attempt_can_advance() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    add(&mut s);
    let j = s.claim_native("receiver-1", 0).unwrap().unwrap();
    assert!(s.claim_native("receiver-1", 0).unwrap().is_none());
    s.bind_native_task(&j.attempt(), "42").unwrap();
    assert!(s.bind_native_task(&j.attempt(), "other").is_err());
    assert!(s
        .complete_native(&j.attempt(), "other", &status(&j, false))
        .is_err());
    s.complete_native(&j.attempt(), "42", &status(&j, false))
        .unwrap();
    drop(s);
    let mut s = Sender::open(&t.0).unwrap();
    assert_eq!(s.checkpoint(j.id).unwrap().unwrap().resources[0].offset, 5);
    assert_eq!(s.job(j.id).unwrap().attempts, 0);
    let next = s.claim_native("receiver-1", 0).unwrap().unwrap();
    assert!(s
        .complete_native(&j.attempt(), "42", &status(&j, true))
        .is_err());
    s.set_paused(true).unwrap();
    assert!(s.bind_native_task(&next.attempt(), "43").is_err());
    s.interrupted(&next.attempt()).unwrap();
    assert!(s.checkpoint(j.id).unwrap().is_none());
    assert!(s.claim_native("receiver-1", 999).unwrap().is_none());
}

#[test]
fn visible_source_status_is_scoped_to_receiver_and_revision() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    let before = s.revision().unwrap();
    let first = add(&mut s);
    assert!(s.revision().unwrap() > before);
    let keys = vec![(first.asset.source_id.clone(), first.asset.revision.clone())];
    assert_eq!(
        s.source_states("receiver-1", &keys).unwrap()[&first.asset.source_id],
        "queued"
    );
    assert!(s.source_states("receiver-2", &keys).unwrap().is_empty());
    assert!(s
        .source_states(
            "receiver-1",
            &[(first.asset.source_id.clone(), "edited".into())]
        )
        .unwrap()
        .is_empty());
    let job = s.claim("receiver-1", 0).unwrap().unwrap();
    s.acknowledge(&job.attempt(), &status(&job, true)).unwrap();
    assert_eq!(
        s.source_states("receiver-1", &keys).unwrap()[&first.asset.source_id],
        "received"
    );
    let summary = s.summary("receiver-1").unwrap();
    assert_eq!(summary["received"], 1);
    assert_eq!(summary["confirmed_bytes"], 10);
    assert_eq!(s.summary("receiver-2").unwrap()["total"], 0);
    assert!(s
        .source_states("receiver-1", &vec![keys[0].clone(); 401])
        .is_err());
}

#[test]
fn filtering_precedes_task_pagination() {
    let t = Temp::new();
    let mut sender = Sender::open(&t.0).unwrap();
    for index in 0..205 {
        let mut a = asset();
        a.source_id = format!("source-{index}");
        sender
            .enqueue(
                "receiver-1",
                a,
                BTreeMap::from([(digest(b"0123456789"), "resource".into())]),
            )
            .unwrap();
    }
    let mut a = asset();
    a.source_id = "other-receiver".into();
    let other = sender
        .enqueue(
            "receiver-2",
            a,
            BTreeMap::from([(digest(b"0123456789"), "resource".into())]),
        )
        .unwrap();
    assert_eq!(
        sender
            .list_filtered(0, 200, Some("receiver-2"), None)
            .unwrap()[0]
            .id,
        other.id
    );
    assert_eq!(
        sender
            .list_filtered(0, 200, Some("receiver-1"), None)
            .unwrap()
            .len(),
        200
    );
    assert_eq!(
        sender
            .list_filtered(200, 200, Some("receiver-1"), None)
            .unwrap()
            .len(),
        5
    );
    let running = sender.claim("receiver-2", 100).unwrap().unwrap();
    sender
        .fail(&running.attempt(), Failure::Network, 100)
        .unwrap();
    assert_eq!(
        sender.list_filtered(0, 200, None, Some("waiting")).unwrap()[0].id,
        other.id
    );
    assert!(sender
        .list_filtered(0, 200, Some("receiver-1"), Some("waiting"))
        .unwrap()
        .is_empty());
}

#[test]
fn verified_connection_recovers_only_its_network_errors_without_resuming_pause() {
    let t = Temp::new();
    let mut s = Sender::open(&t.0).unwrap();
    let mut cases = Vec::new();
    for (n, failure) in [
        Failure::Network,
        Failure::Authentication,
        Failure::Integrity,
        Failure::SourceUnavailable,
        Failure::LowSpace,
    ]
    .into_iter()
    .enumerate()
    {
        let mut a = asset();
        a.source_id = format!("route-{n}");
        let sources = BTreeMap::from([(a.resources[0].sha256.clone(), "fixture".into())]);
        let job = s.enqueue("receiver-1", a, sources).unwrap();
        let running = s.claim("receiver-1", 0).unwrap().unwrap();
        assert_eq!(job.id, running.id);
        s.acknowledge(&running.attempt(), &status(&running, false))
            .unwrap();
        s.fail(&running.attempt(), failure, 10000).unwrap();
        cases.push(s.job(job.id).unwrap());
    }
    let other = s
        .enqueue(
            "receiver-2",
            asset(),
            BTreeMap::from([(asset().resources[0].sha256.clone(), "fixture".into())]),
        )
        .unwrap();
    let running = s.claim("receiver-2", 0).unwrap().unwrap();
    s.fail(&running.attempt(), Failure::Authentication, 10000)
        .unwrap();
    s.pause_job(cases[0].id).unwrap();
    s.set_paused(true).unwrap();
    assert_eq!(s.recover_connection("receiver-1").unwrap(), 1);
    assert!(s.paused().unwrap());
    assert!(s.claim("receiver-1", i64::MAX).unwrap().is_none());
    assert_eq!(s.job(cases[0].id).unwrap().state, JobState::Paused);
    let auth = s.job(cases[1].id).unwrap();
    assert_eq!(auth.state, JobState::Queued);
    assert_eq!(auth.confirmed_bytes, 5);
    assert!(auth.generation > cases[1].generation);
    assert_eq!(s.job(other.id).unwrap().state, JobState::Failed);
    for old in &cases[2..] {
        assert_eq!(s.job(old.id).unwrap().state, old.state);
    }
    assert_eq!(s.recover_connection("receiver-1").unwrap(), 0);
}
