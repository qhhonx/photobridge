use photobridge_pixel::{write_jpeg_motion, write_jpeg_motion_with_burst_and_video_mime};
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

#[test]
fn jpeg_live_photo_keeps_original_mov() {
    let root = workdir();
    let image = root.join("still.jpg");
    let movie = root.join("paired.mov");
    let output = root.join("output.jpg");
    let jpeg = [
        0xff, 0xd8, 0xff, 0xe0, 0, 4, 1, 2, 0xff, 0xda, 0, 2, 0xff, 0xd9,
    ];
    let mov = b"\0\0\0\x14ftypqt  \0\0\0\0qt  ";
    std::fs::write(&image, jpeg).unwrap();
    std::fs::write(&movie, mov).unwrap();
    write_jpeg_motion_with_burst_and_video_mime(
        &image,
        &movie,
        &output,
        None,
        None,
        "video/quicktime",
    )
    .unwrap();
    let result = std::fs::read(output).unwrap();
    assert!(result.windows(mov.len()).any(|window| window == mov));
    assert!(result.ends_with(b"SEFT"));
    assert!(String::from_utf8_lossy(&result).contains("Item:Mime=\"video/quicktime\""));
    assert_eq!(std::fs::read(image).unwrap(), jpeg);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_jpeg_live_photo_when_available() {
    let Ok(dir) = std::env::var("PHOTOBRIDGE_LIVE_PHOTO_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(dir);
    let still = root.join("still.jpg");
    if !still.exists() {
        return;
    }
    let video = root.join("paired.mov");
    let output = root.join("rust-jpeg-mov.jpg");
    let _ = std::fs::remove_file(&output);
    write_jpeg_motion_with_burst_and_video_mime(
        &still,
        &video,
        &output,
        None,
        None,
        "video/quicktime",
    )
    .unwrap();
    let result = std::fs::read(&output).unwrap();
    let original_video = std::fs::read(&video).unwrap();
    assert!(result
        .windows(original_video.len())
        .any(|window| window == original_video));
    assert!(result.ends_with(b"SEFT"));
    assert!(String::from_utf8_lossy(&result).contains("Item:Mime=\"video/quicktime\""));
}

const XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const GAIN_MAP: [u8; 14] = [
    0xff, 0xd8, 0xff, 0xe0, 0, 4, 9, 9, 0xff, 0xda, 0, 2, 0xff, 0xd9,
];

fn workdir() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "photobridge-motion-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0xff, marker];
    out.extend(((payload.len() + 2) as u16).to_be_bytes());
    out.extend(payload);
    out
}

/// The XMP packet Android's Ultra HDR encoder writes for `Bitmap.compress(JPEG)`.
fn gain_map_xmp(length: usize) -> Vec<u8> {
    let mut packet = XMP.to_vec();
    packet.extend(
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 5.1.2">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description
        xmlns:Container="http://ns.google.com/photos/1.0/container/"
        xmlns:Item="http://ns.google.com/photos/1.0/container/item/"
        xmlns:hdrgm="http://ns.adobe.com/hdr-gain-map/1.0/"
        hdrgm:Version="1.0">
      <Container:Directory>
        <rdf:Seq>
          <rdf:li rdf:parseType="Resource">
            <Container:Item Item:Semantic="Primary" Item:Mime="image/jpeg"/>
          </rdf:li>
          <rdf:li rdf:parseType="Resource">
            <Container:Item Item:Semantic="GainMap" Item:Mime="image/jpeg" Item:Length="{length}"/>
          </rdf:li>
        </rdf:Seq>
      </Container:Directory>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#
        )
        .as_bytes(),
    );
    segment(0xe1, &packet)
}

/// Big-endian MPF APP2 with a primary entry and a gain map entry.
fn mpf(primary_size: u32, gain_offset: u32, gain_size: u32) -> Vec<u8> {
    let mut p = b"MPF\0MM\0*".to_vec();
    p.extend(8u32.to_be_bytes()); // IFD
    p.extend(1u16.to_be_bytes());
    p.extend(0xb002u16.to_be_bytes());
    p.extend(7u16.to_be_bytes());
    p.extend(32u32.to_be_bytes());
    p.extend(26u32.to_be_bytes()); // entries after IFD (8 + 2 + 12 + 4)
    p.extend(0u32.to_be_bytes()); // next IFD
    for (attr, size, offset) in [
        (0x0003_0000u32, primary_size, 0u32),
        (0, gain_size, gain_offset),
    ] {
        p.extend(attr.to_be_bytes());
        p.extend(size.to_be_bytes());
        p.extend(offset.to_be_bytes());
        p.extend([0u8; 4]);
    }
    segment(0xe2, &p)
}

/// An Ultra HDR JPEG as Android produces it. With `exif`, an EXIF segment is
/// inserted after encoding the way androidx ExifInterface does: the MP header and
/// gain map move together (offsets stay exact) but the primary size entry is stale.
fn ultra_hdr(xmp_after_mpf: bool, declared_gain: usize, exif: bool) -> Vec<u8> {
    let xmp = gain_map_xmp(declared_gain);
    let placeholder = mpf(0, 0, 0);
    let scan = [0xff, 0xda, 0, 2, 0xff, 0xd9];
    let primary_len = 2 + xmp.len() + placeholder.len() + scan.len();
    let mpf_at = 2 + if xmp_after_mpf { 0 } else { xmp.len() };
    let header = mpf_at + 8;
    let mut out = vec![0xff, 0xd8];
    let table = mpf(
        primary_len as u32,
        (primary_len - header) as u32,
        GAIN_MAP.len() as u32,
    );
    if xmp_after_mpf {
        out.extend(&table);
        out.extend(&xmp);
    } else {
        out.extend(&xmp);
        out.extend(&table);
    }
    out.extend(scan);
    assert_eq!(out.len(), primary_len);
    out.extend(GAIN_MAP);
    if exif {
        let mut segment = segment(0xe1, b"Exif\0\0MM\0*\0\0\0\x08\0\0");
        segment.extend(out.split_off(2));
        out.extend(segment);
    }
    out
}

/// Returns (MP header position, [(size, offset)]) of the only MPF segment before SOS.
fn read_mpf(bytes: &[u8]) -> (usize, Vec<(u32, u32)>) {
    let mut at = 2;
    loop {
        assert_eq!(bytes[at], 0xff);
        let marker = bytes[at + 1];
        assert_ne!(marker, 0xda, "no MPF segment");
        let len = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        if marker == 0xe2 && bytes[at + 4..].starts_with(b"MPF\0") {
            let header = at + 8;
            let be = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
            let entries = header + 26;
            return (
                header,
                (0..2)
                    .map(|j| (be(entries + j * 16 + 4), be(entries + j * 16 + 8)))
                    .collect(),
            );
        }
        at += 2 + len;
    }
}

#[test]
fn ultra_hdr_gain_map_survives_motion_packaging() {
    for (xmp_after_mpf, exif) in [(false, false), (true, false), (false, true), (true, true)] {
        let root = workdir();
        let video = b"\0\0\0\x14ftypisom\0\0\0\0isom";
        let (image, movie, out) = (
            root.join("still.jpg"),
            root.join("video.mp4"),
            root.join("output.jpg"),
        );
        let source = ultra_hdr(xmp_after_mpf, GAIN_MAP.len(), exif);
        std::fs::write(&image, &source).unwrap();
        std::fs::write(&movie, video).unwrap();
        write_jpeg_motion(&image, &movie, &out, None).unwrap();
        let bytes = std::fs::read(&out).unwrap();

        assert!(bytes.ends_with(video));
        let gain_at = bytes.len() - video.len() - GAIN_MAP.len();
        assert_eq!(&bytes[gain_at..bytes.len() - video.len()], GAIN_MAP);
        assert_eq!(&bytes[gain_at - 2..gain_at], &[0xff, 0xd9]);
        let packets = bytes.windows(XMP.len()).filter(|w| *w == XMP).count();
        assert_eq!(packets, 1, "exactly one XMP packet");

        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(r#"hdrgm:Version="1.0""#));
        assert!(text.contains(r#"GCamera:MotionPhoto="1""#));
        let primary = text.find(r#"Item:Semantic="Primary""#).unwrap();
        let gain = text
            .find(&format!(
                r#"Item:Semantic="GainMap" Item:Length="{}""#,
                GAIN_MAP.len()
            ))
            .unwrap();
        let motion = text
            .find(&format!(
                r#"Item:Semantic="MotionPhoto" Item:Length="{}""#,
                video.len()
            ))
            .unwrap();
        assert!(primary < gain && gain < motion);

        let (header, entries) = read_mpf(&bytes);
        assert_eq!(
            entries[0],
            (gain_at as u32, 0),
            "primary size follows the new packet"
        );
        assert_eq!(entries[1].0 as usize, GAIN_MAP.len());
        assert_eq!(
            header + entries[1].1 as usize,
            gain_at,
            "gain map offset still resolves"
        );
        assert_eq!(std::fs::read(&image).unwrap(), source);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn unknown_or_inconsistent_xmp_is_still_refused() {
    let root = workdir();
    let video = b"\0\0\0\x14ftypisom\0\0\0\0isom";
    let movie = root.join("video.mp4");
    std::fs::write(&movie, video).unwrap();
    let mut plain = vec![0xff, 0xd8];
    let mut packet = XMP.to_vec();
    packet.extend(br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about=""/></rdf:RDF></x:xmpmeta>"#);
    plain.extend(segment(0xe1, &packet));
    plain.extend([0xff, 0xda, 0, 2, 0xff, 0xd9]);
    for (name, source) in [
        ("plain.jpg", plain),
        ("mismatch.jpg", ultra_hdr(false, GAIN_MAP.len() + 1, true)),
    ] {
        let image = root.join(name);
        let out = root.join(format!("{name}.out"));
        std::fs::write(&image, source).unwrap();
        assert!(
            write_jpeg_motion(&image, &movie, &out, None).is_err(),
            "{name}"
        );
        assert!(!out.exists() && !out.with_extension("motion.partial").exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}
