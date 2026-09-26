use photobridge_folder_source::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "pb-folder-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(p.join("source/nested")).unwrap();
        Self(p)
    }
    fn root(&self) -> PathBuf {
        self.0.join("source")
    }
    fn index(&self) -> Index {
        Index::open(&self.0.join("index.sqlite3")).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn clock() -> u64 {
    millis(SystemTime::now())
}
fn scan(i: &mut Index, source: &str, root: &Path, g: u64) {
    i.begin(source, root, g).unwrap();
    for _ in 0..10000 {
        if !i.step(g, 2).unwrap().scanning {
            return;
        }
    }
    panic!("unbounded scan")
}
#[test]
fn recursive_incremental_and_restart_preserve_target_scoped_receipts() {
    let t = Temp::new();
    fs::write(t.root().join("nested/a.jpg"), b"original").unwrap();
    fs::write(t.root().join("note.txt"), b"not media").unwrap();
    let now = clock();
    let mut i = t.index();
    i.begin("a", &t.root(), now).unwrap();
    assert!(i.step(now, 1).unwrap().scanning);
    while i.step(now, 2).unwrap().scanning {}
    assert_eq!(i.summary("a", 0).unwrap().files, 1);
    assert!(i.candidates("a", "r", now, 10).unwrap().is_empty()); // 10 second settling window
    let e = i.candidates("a", "r", now + 11000, 10).unwrap().remove(0);
    i.mark("a", &e.relative, "r", &e.revision, 7).unwrap();
    assert!(i.candidates("a", "r", now + 11000, 10).unwrap().is_empty());
    assert_eq!(
        i.candidates("a", "other", now + 11000, 10).unwrap().len(),
        1
    );
    drop(i);
    let mut i = t.index();
    scan(&mut i, "a", &t.root(), now + 12000);
    assert!(i.candidates("a", "r", now + 23000, 10).unwrap().is_empty());
    assert_eq!(i.page("a", 0, "r").unwrap()[0].job_id, Some(7));
    fs::write(t.root().join("nested/a.jpg"), b"changed original").unwrap();
    scan(&mut i, "a", &t.root(), now + 24000);
    assert!(i.candidates("a", "r", now + 24000, 10).unwrap().is_empty());
    assert_eq!(i.candidates("a", "r", now + 35000, 10).unwrap().len(), 1);
    assert!(i.mark("a", &e.relative, "r", &e.revision, 7).is_err());
}
#[test]
fn rename_does_not_requeue_on_unix_and_missing_source_is_not_deleted() {
    let t = Temp::new();
    fs::write(t.root().join("a.jpg"), b"original").unwrap();
    let mut i = t.index();
    let now = clock();
    scan(&mut i, "a", &t.root(), now);
    let e = i.candidates("a", "r", now + 11000, 10).unwrap().remove(0);
    i.mark("a", &e.relative, "r", &e.revision, 7).unwrap();
    fs::rename(t.root().join("a.jpg"), t.root().join("renamed.jpg")).unwrap();
    scan(&mut i, "a", &t.root(), now + 12000);
    #[cfg(unix)]
    assert!(i.candidates("a", "r", now + 23000, 10).unwrap().is_empty());
    assert!(i.begin("a", &t.root().join("absent"), now + 24000).is_err());
    assert_eq!(i.summary("a", 0).unwrap().files, 1);
    fs::remove_file(t.root().join("renamed.jpg")).unwrap();
    scan(&mut i, "a", &t.root(), now + 25000);
    assert_eq!(i.summary("a", 0).unwrap().files, 0);
}
#[test]
fn baseline_excludes_only_existing_and_ignore_can_be_retried() {
    let t = Temp::new();
    fs::write(t.root().join("a.jpg"), b"a").unwrap();
    let now = clock();
    let mut i = t.index();
    scan(&mut i, "a", &t.root(), now);
    i.baseline("a", "r").unwrap();
    assert!(i.candidates("a", "r", now + 11000, 10).unwrap().is_empty());
    fs::write(t.root().join("b.mov"), b"video").unwrap();
    scan(&mut i, "a", &t.root(), now + 12000);
    let e = i.candidates("a", "r", now + 23000, 10).unwrap().remove(0);
    i.ignore("a", &e.relative, &e.revision).unwrap();
    assert!(i.candidates("a", "r", now + 23000, 10).unwrap().is_empty());
    i.retry_ignored("a").unwrap();
    assert_eq!(i.candidates("a", "r", now + 23000, 10).unwrap().len(), 1);
}
#[test]
fn cancelled_scan_never_hides_previous_inventory() {
    let t = Temp::new();
    for n in 0..30 {
        fs::write(t.root().join(format!("{n}.jpg")), b"a").unwrap();
    }
    let mut i = t.index();
    let now = clock();
    scan(&mut i, "a", &t.root(), now);
    i.begin("a", &t.root(), now + 100).unwrap();
    i.step(now + 100, 1).unwrap();
    i.cancel();
    assert_eq!(i.summary("a", 0).unwrap().files, 30);
}
#[test]
fn unsafe_paths_and_links_are_rejected() {
    let t = Temp::new();
    fs::write(t.root().join("a.jpg"), b"a").unwrap();
    assert!(resolve(&t.root(), "../outside.jpg").is_err());
    assert!(resolve(&t.root(), "/absolute.jpg").is_err());
    assert!(resolve(&t.root(), "nested/../a.jpg").is_err());
    assert!(resolve(&t.root(), "a.jpg").is_ok());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.root().join("a.jpg"), t.root().join("link.jpg")).unwrap();
        assert!(resolve(&t.root(), "link.jpg").is_err());
        let mut i = t.index();
        scan(&mut i, "a", &t.root(), clock());
        assert_eq!(i.summary("a", 0).unwrap().files, 1);
    }
    assert_eq!(millis(UNIX_EPOCH), 0);
}

#[test]
fn subtree_events_do_not_hide_other_folders_and_removed_subtree_is_detected() {
    let t = Temp::new();
    fs::create_dir(t.root().join("other")).unwrap();
    fs::write(t.root().join("other/kept.jpg"), b"kept").unwrap();
    fs::write(t.root().join("nested/a.jpg"), b"a").unwrap();
    let now = clock();
    let mut i = t.index();
    scan(&mut i, "a", &t.root(), now);
    fs::write(t.root().join("nested/b.jpg"), b"b").unwrap();
    i.begin_scoped("a", &t.root(), now + 100, &["nested".into()])
        .unwrap();
    while i.step(now + 100, 2).unwrap().scanning {}
    assert_eq!(i.summary("a", 0).unwrap().files, 3);
    fs::remove_dir_all(t.root().join("nested")).unwrap();
    i.begin_scoped("a", &t.root(), now + 200, &["nested".into()])
        .unwrap();
    while i.step(now + 200, 2).unwrap().scanning {}
    assert_eq!(i.summary("a", 0).unwrap().files, 1);
    assert_eq!(i.page("a", 0, "r").unwrap()[0].relative, "other/kept.jpg");
}

#[test]
fn temporary_capacity_deferral_keeps_one_off_work_pending_and_allows_other_files() {
    let t = Temp::new();
    fs::write(t.root().join("a.jpg"), b"a").unwrap();
    fs::write(t.root().join("b.jpg"), b"b").unwrap();
    let now = clock();
    let mut i = t.index();
    scan(&mut i, "a", &t.root(), now);
    let e = i.candidates("a", "r", now + 11000, 10).unwrap().remove(0);
    i.defer("a", &e.relative, &e.revision, now + 70000).unwrap();
    assert_eq!(i.candidates("a", "r", now + 12000, 10).unwrap().len(), 1);
    assert_eq!(i.pending("a", "r").unwrap(), 2);
    assert_eq!(i.candidates("a", "r", now + 71000, 10).unwrap().len(), 2);
}

#[test]
fn initial_baseline_is_independent_of_pairing_and_explicit_import_preserves_receipts() {
    let t = Temp::new();
    fs::write(t.root().join("old.jpg"), b"old").unwrap();
    let now = clock();
    let mut i = t.index();
    scan(&mut i, "a", &t.root(), now);
    i.baseline("a", "@baseline").unwrap();
    assert!(i
        .candidates("a", "future-receiver", now + 11000, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        i.page("a", 0, "future-receiver").unwrap()[0].job_id,
        Some(0)
    );
    fs::write(t.root().join("new.jpg"), b"new").unwrap();
    scan(&mut i, "a", &t.root(), now + 12000);
    let e = i.candidates("a", "r", now + 23000, 10).unwrap().remove(0);
    assert_eq!(e.relative, "new.jpg");
    i.mark("a", &e.relative, "r", &e.revision, 8).unwrap();
    i.include_existing("a").unwrap();
    assert_eq!(
        i.candidates("a", "r", now + 23000, 10).unwrap()[0].relative,
        "old.jpg"
    );
    assert_eq!(i.job_id("a", "new.jpg", "r").unwrap(), Some(8));
}

#[test]
fn children_are_complete_scoped_and_independently_paginated() {
    let t = Temp::new();
    for folder in ["a_%", "a_X", "nested/deeper"] {
        fs::create_dir_all(t.root().join(folder)).unwrap();
    }
    for n in 0..205 {
        fs::write(t.root().join(format!("a_%/{n:03}.jpg")), b"image").unwrap();
    }
    fs::write(t.root().join("a_X/other.jpg"), b"image").unwrap();
    fs::write(t.root().join("nested/deeper/video.mov"), b"video").unwrap();
    fs::write(t.root().join("root.jpg"), b"image").unwrap();
    let mut index = t.index();
    scan(&mut index, "source", &t.root(), clock());
    let root = index.children("source", "", 0, "r").unwrap();
    assert_eq!(
        root.rows
            .iter()
            .map(|c| c.relative.as_str())
            .collect::<Vec<_>>(),
        vec!["a_%", "a_X", "nested", "root.jpg"]
    );
    assert!(!root.has_more);
    assert!(root.rows[..3]
        .iter()
        .all(|c| c.is_directory && c.entry.is_none()));
    let mut files = Vec::new();
    for (offset, expected, more) in [(0, 100, true), (100, 100, true), (200, 5, false)] {
        let page = index.children("source", "a_%", offset, "r").unwrap();
        assert_eq!(page.rows.len(), expected);
        assert_eq!(page.has_more, more);
        files.extend(page.rows.into_iter().map(|c| c.relative));
    }
    files.sort();
    files.dedup();
    assert_eq!(files.len(), 205);
    assert!(files.iter().all(|f| f.starts_with("a_%/")));
    assert_eq!(
        index.children("source", "nested", 0, "r").unwrap().rows[0].relative,
        "nested/deeper"
    );
    assert!(index
        .children("other-source", "", 0, "r")
        .unwrap()
        .rows
        .is_empty());
    assert!(index.children("source", "../a_%", 0, "r").is_err());
    assert!(index.children("source", "/a_%", 0, "r").is_err());
    fs::remove_file(t.root().join("root.jpg")).unwrap();
    scan(&mut index, "source", &t.root(), clock() + 1);
    assert!(index
        .children("source", "", 0, "r")
        .unwrap()
        .rows
        .iter()
        .all(|c| c.relative != "root.jpg"));
}

#[test]
fn preview_lookup_follows_identity_and_keeps_receipts_scoped() {
    let t = Temp::new();
    fs::write(t.root().join("nested/photo.jpg"), b"original").unwrap();
    fs::write(t.root().join("A.MOV"), b"companion video").unwrap();
    let mut index = t.index();
    let now = clock();
    scan(&mut index, "source", &t.root(), now);
    let original = index.entry("source", "nested/photo.jpg").unwrap();
    index
        .mark("source", &original.relative, "r", &original.revision, 42)
        .unwrap();
    let child = index
        .children("source", "nested", 0, "r")
        .unwrap()
        .rows
        .remove(0);
    assert_eq!(child.entry.unwrap().job_id, Some(42));
    assert_eq!(
        index
            .children("source", "nested", 0, "other-receiver")
            .unwrap()
            .rows[0]
            .entry
            .as_ref()
            .unwrap()
            .job_id,
        None
    );
    assert!(index
        .preview_entry("other-source", 42, &original.source_id)
        .unwrap()
        .is_none());
    assert!(index
        .preview_entry("source", 99, &original.source_id)
        .unwrap()
        .is_none());
    #[cfg(unix)]
    {
        fs::rename(
            t.root().join("nested/photo.jpg"),
            t.root().join("renamed.jpg"),
        )
        .unwrap();
        scan(&mut index, "source", &t.root(), now + 1);
        let preview = index
            .preview_entry("source", 42, &original.source_id)
            .unwrap()
            .unwrap();
        assert_eq!(preview.relative, "renamed.jpg");
        assert_eq!(preview.source_id, original.source_id);
        assert_eq!(preview.revision, original.revision);
        fs::write(t.root().join("renamed.jpg"), b"different longer contents").unwrap();
        scan(&mut index, "source", &t.root(), now + 2);
        assert_ne!(
            index
                .preview_entry("source", 42, &original.source_id)
                .unwrap()
                .unwrap()
                .revision,
            original.revision
        );
        fs::remove_file(t.root().join("renamed.jpg")).unwrap();
        scan(&mut index, "source", &t.root(), now + 3);
        assert!(index
            .preview_entry("source", 42, &original.source_id)
            .unwrap()
            .is_none());
    }
}

#[test]
fn root_folder_pagination_includes_directories_after_the_first_page() {
    let t = Temp::new();
    for n in 0..105 {
        let folder = t.root().join(format!("folder-{n:03}"));
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("photo.jpg"), b"photo").unwrap();
    }
    let mut index = t.index();
    scan(&mut index, "source", &t.root(), clock());
    let first = index.children("source", "", 0, "r").unwrap();
    let last = index.children("source", "", 100, "r").unwrap();
    assert_eq!(first.rows.len(), 100);
    assert!(first.has_more);
    assert_eq!(last.rows.len(), 5);
    assert!(!last.has_more);
    assert_eq!(last.rows.last().unwrap().relative, "folder-104");
    assert!(first
        .rows
        .iter()
        .chain(&last.rows)
        .all(|row| row.is_directory));
}
