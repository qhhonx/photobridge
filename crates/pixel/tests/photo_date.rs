use little_exif::{exif_tag::ExifTag, metadata::Metadata};
use photobridge_pixel::write_photo_date;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQ: AtomicU64 = AtomicU64::new(0);
#[test]
fn heic_capture_date_roundtrips_without_touching_original_or_overwriting_output() {
    let root = std::env::temp_dir().join(format!(
        "photobridge-date-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.heic");
    let output = root.join("dated.heic");
    let bytes = include_bytes!("fixtures/date.heic");
    fs::write(&source, bytes).unwrap();
    let date = "2026:08:15 02:41:41";
    assert!(write_photo_date(&source, &source, date, 0).is_err());
    assert!(write_photo_date(&source, &output, "invalid", 0).is_err());
    assert!(!output.exists());
    write_photo_date(&source, &output, date, 0).unwrap();
    let metadata = Metadata::new_from_path(&output).unwrap();
    assert!(
        matches!(metadata.get_tag(&ExifTag::DateTimeOriginal(String::new())).next(), Some(ExifTag::DateTimeOriginal(v)) if v == date)
    );
    assert!(
        matches!(metadata.get_tag(&ExifTag::OffsetTimeOriginal(String::new())).next(), Some(ExifTag::OffsetTimeOriginal(v)) if v == "+00:00")
    );
    let before = fs::read(&output).unwrap();
    assert!(write_photo_date(&source, &output, date, 0).is_err());
    assert_eq!(fs::read(&output).unwrap(), before);
    assert_eq!(fs::read(&source).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}
