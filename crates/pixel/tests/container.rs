use photobridge_pixel::write_jpeg_motion;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(0);
#[test]
fn jpeg_container_has_exact_video_tail_and_no_invented_presentation_time() {
    let root = std::env::temp_dir().join(format!(
        "photobridge-motion-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    // Structural fixture only: actual codec/playback validation is native-device work.
    let jpeg = [
        0xff, 0xd8, 0xff, 0xe0, 0, 4, 1, 2, 0xff, 0xda, 0, 2, 0xff, 0xd9,
    ];
    let video = b"\0\0\0\x14ftypisom\0\0\0\0isom";
    let image = root.join("still.jpg");
    let movie = root.join("video.mp4");
    let out = root.join("output.jpg");
    std::fs::write(&image, jpeg).unwrap();
    std::fs::write(&movie, video).unwrap();
    write_jpeg_motion(&image, &movie, &out, None).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.ends_with(video));
    assert_eq!(&bytes[..4], &[0xff, 0xd8, 0xff, 0xe1]);
    let packet = String::from_utf8_lossy(&bytes);
    assert!(packet.contains("GCamera:MotionPhoto=\"1\""));
    assert!(packet.contains("Item:Length=\"20\""));
    assert!(!packet.contains("PresentationTimestampUs"));
    assert_eq!(std::fs::read(&image).unwrap(), jpeg);
    assert!(write_jpeg_motion(&out, &movie, &root.join("bad.jpg"), None).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
