use photobridge_pixel::{
    write_heic_motion_with_burst, write_heic_motion_with_burst_and_video_mime,
};
use std::{fs, path::PathBuf};

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend(kind);
    out.extend(body);
    out
}

#[test]
fn heic_and_mov_payloads_are_preserved() {
    let root = std::env::temp_dir().join(format!("photobridge-motion-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let still = root.join("still.heic");
    let video = root.join("paired.mov");
    let output = root.join("result.heic");
    let ftyp = atom(b"ftyp", b"heic\0\0\0\0mif1heic");
    let pitm = atom(b"pitm", &[0, 0, 0, 0, 0, 1]);
    let infe = atom(b"infe", b"\x02\0\0\x01\0\x01\0\0hvc1\0");
    let mut iinf_body = vec![0, 0, 0, 0, 0, 1];
    iinf_body.extend(infe);
    let iinf = atom(b"iinf", &iinf_body);
    let iref = atom(b"iref", &[0, 0, 0, 0]);
    let mut iloc_body = vec![1, 0, 0, 0, 0x44, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1];
    iloc_body.extend([0, 0, 0, 0, 0, 0, 0, 5]);
    let mut meta_body = vec![0, 0, 0, 0];
    meta_body.extend(pitm);
    meta_body.extend(iinf);
    meta_body.extend(iref);
    meta_body.extend(atom(b"iloc", &iloc_body));
    let mut meta = atom(b"meta", &meta_body);
    let image_offset = (ftyp.len() + meta.len() + 8) as u32;
    let iloc_start = meta.windows(4).position(|w| w == b"iloc").unwrap() - 4;
    meta[iloc_start + 24..iloc_start + 28].copy_from_slice(&image_offset.to_be_bytes());
    let pixels = b"abcde";
    let mut image = ftyp;
    image.extend(meta);
    image.extend(atom(b"mdat", pixels));
    fs::write(&still, &image).unwrap();
    let movie = atom(b"ftyp", b"qt  \0\0\0\0qt  ");
    fs::write(&video, &movie).unwrap();
    write_heic_motion_with_burst(&still, &video, &output, None).unwrap();
    let result = fs::read(&output).unwrap();
    assert_eq!(fs::read(&still).unwrap(), image);
    assert_eq!(fs::read(&video).unwrap(), movie);
    assert!(result.windows(pixels.len()).any(|w| w == pixels));
    assert!(result.windows(movie.len()).any(|w| w == movie));
    assert!(result
        .windows(b"MotionPhoto".len())
        .any(|w| w == b"MotionPhoto"));
    assert!(result.ends_with(b"SEFT"));
    let mp4_output = root.join("result-mp4.heic");
    write_heic_motion_with_burst_and_video_mime(&still, &video, &mp4_output, None, "video/mp4")
        .unwrap();
    assert!(String::from_utf8_lossy(&fs::read(&mp4_output).unwrap())
        .contains("Item:Mime=\"video/mp4\""));
    assert!(write_heic_motion_with_burst(&still, &video, &output, None).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_heic_fixture_when_available() {
    // Run locally with an exported Live Photo pair; never commit personal media.
    let Ok(dir) = std::env::var("PHOTOBRIDGE_LIVE_PHOTO_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(dir);
    let output = root.join("rust-no-transcode.heic");
    let _ = fs::remove_file(&output);
    write_heic_motion_with_burst(
        &root.join("still.heic"),
        &root.join("paired.mov"),
        &output,
        None,
    )
    .unwrap();
    assert!(output.metadata().unwrap().len() > root.join("still.heic").metadata().unwrap().len());
}
