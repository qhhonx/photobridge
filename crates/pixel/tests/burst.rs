use photobridge_core::BurstMetadata;
use photobridge_pixel::{write_jpeg_burst, write_jpeg_motion_with_burst};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};
static SEQ: AtomicU64 = AtomicU64::new(0);
fn jpeg(xml: Option<&str>) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    // These are structural fixtures; Android instrumentation uses actual JPEGs.
    bytes.extend([0xff, 0xe1, 0, 10]);
    bytes.extend(b"Exif\0\0AB");
    bytes.extend([0xff, 0xe2, 0, 5, 1, 2, 3]);
    if let Some(xml) = xml {
        let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        xmp.extend(xml.as_bytes());
        bytes.extend([0xff, 0xe1]);
        bytes.extend(((xmp.len() + 2) as u16).to_be_bytes());
        bytes.extend(xmp);
    }
    bytes.extend([0xff, 0xda, 0, 2, 9, 8, 7, 0xff, 0xd9]);
    bytes
}
#[test]
fn burst_copy_preserves_original_pixels_and_xmp_and_is_idempotent() {
    let root = std::env::temp_dir().join(format!(
        "photobridge-burst-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let burst = BurstMetadata::from_identifier("fixture-group", true).unwrap();
    for (i,xml) in [None,Some(r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><r:Description xmlns:dc="http://purl.org/dc/elements/1.1/" dc:description="山与海 &amp; sky"/></r:RDF></x:xmpmeta>"#),Some(r#"<r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#"/>"#)].into_iter().enumerate() {
        let source=root.join(format!("source-{i}.jpg"));let out=root.join(format!("out-{i}.jpg"));let again=root.join(format!("again-{i}.jpg"));
        let original=jpeg(xml);fs::write(&source,&original).unwrap();
        write_jpeg_burst(&source,&out,&burst).unwrap();let bytes=fs::read(&out).unwrap();
        assert_eq!(fs::read(&source).unwrap(),original);
        assert!(bytes.ends_with(&[0xff,0xda,0,2,9,8,7,0xff,0xd9]));
        assert!(bytes.windows(8).any(|b|b==b"Exif\0\0AB"));assert!(bytes.windows(7).any(|b|b==[0xff,0xe2,0,5,1,2,3]));
        let text=String::from_utf8_lossy(&bytes);assert!(text.contains(&burst.group_id));assert!(text.contains("GCamera:BurstPrimary=\"1\""));
        if i==1 {assert!(text.contains("山与海 &amp; sky"));}
        write_jpeg_burst(&out,&again,&burst).unwrap();assert_eq!(fs::read(again).unwrap(),bytes);
        assert!(write_jpeg_burst(&source,&source,&burst).is_err());assert_eq!(fs::read(source).unwrap(),original);
    }
    let source = root.join("source.jpg");
    fs::write(&source, jpeg(None)).unwrap();
    let out = root.join("collision.jpg");
    let partial = out.with_extension("burst.partial");
    fs::write(&partial, b"other operation").unwrap();
    assert!(write_jpeg_burst(&source, &out, &burst).is_err());
    assert_eq!(fs::read(partial).unwrap(), b"other operation");
    for (i,xml) in ["<!DOCTYPE x><r:RDF xmlns:r=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"/>","<broken>","<x/>",r#"<r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><r:Description xmlns:g="http://ns.google.com/photos/1.0/camera/" g:BurstID="conflicting"/></r:RDF>"#].iter().enumerate() {
        fs::write(&source,jpeg(Some(xml))).unwrap();let bad=root.join(format!("bad-{i}.jpg"));assert!(write_jpeg_burst(&source,&bad,&burst).is_err());assert!(!bad.exists());
    }
    fs::write(&source, jpeg(None)).unwrap();
    let video = root.join("video.mp4");
    let movie = b"\0\0\0\x14ftypisom\0\0\0\0isom";
    fs::write(&video, movie).unwrap();
    let motion = root.join("motion.jpg");
    write_jpeg_motion_with_burst(&source, &video, &motion, None, Some(&burst)).unwrap();
    let bytes = fs::read(motion).unwrap();
    assert!(bytes.ends_with(movie));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("GCamera:MotionPhoto=\"1\""));
    assert!(text.contains(&burst.group_id));
    let occupied = root.join("occupied.jpg");
    let occupied_partial = occupied.with_extension("motion.partial");
    fs::write(&occupied_partial, b"other conversion").unwrap();
    assert!(write_jpeg_motion_with_burst(&source, &video, &occupied, None, Some(&burst)).is_err());
    assert_eq!(fs::read(occupied_partial).unwrap(), b"other conversion");
    let mut multi = jpeg(None);
    multi.splice(2..2, [0xff, 0xe2, 0, 6, b'M', b'P', b'F', 0]);
    fs::write(&source, multi).unwrap();
    assert!(write_jpeg_burst(&source, &root.join("multi.jpg"), &burst).is_err());
    fs::remove_dir_all(root).unwrap();
}
