//! Pixel-specific output planning. No Pixel policies leak into transport.
//! A native Android host must implement conversion and MediaStore publication.
mod burst;
mod photo_date;
pub use photo_date::write_photo_date;
pub mod photos_cleanup;
pub mod photos_probe;
pub use burst::write_jpeg_burst;
use photobridge_core::{
    Asset, AssetKind, BurstMetadata, Error, Result, TargetPlan, TargetProcessor,
};

#[derive(Default)]
pub struct PixelTarget {
    pub motion_conversion_available: bool,
}
impl TargetProcessor for PixelTarget {
    fn plan(&self, asset: &Asset) -> Result<TargetPlan> {
        asset.validate()?;
        match asset.kind {
            AssetKind::Motion if !self.motion_conversion_available => {
                Err(Error::Unsupported("pixel.motion_photo".into()))
            }
            AssetKind::Motion => Ok(TargetPlan::GenerateMotionPhoto),
            AssetKind::Photo if BurstMetadata::from_fields(&asset.metadata)?.is_some() => {
                Ok(TargetPlan::GenerateBurstPhoto)
            }
            _ => Ok(TargetPlan::PublishOriginals),
        }
    }
}

/// Package a native-decoded JPEG and native-encoded MP4 into a Motion Photo.
/// Original resources remain in the receiver store. The host owns codec support;
/// this function owns the container layout. None uses the format's midpoint rule.
pub fn write_jpeg_motion(
    jpeg: &std::path::Path,
    mp4: &std::path::Path,
    output: &std::path::Path,
    timestamp_us: Option<u64>,
) -> photobridge_core::Result<()> {
    write_jpeg_motion_with_burst(jpeg, mp4, output, timestamp_us, None)
}
pub fn write_jpeg_motion_with_burst(
    jpeg: &std::path::Path,
    mp4: &std::path::Path,
    output: &std::path::Path,
    timestamp_us: Option<u64>,
    burst: Option<&BurstMetadata>,
) -> Result<()> {
    if jpeg == output
        || mp4 == output
        || (output.exists()
            && (jpeg.canonicalize()? == output.canonicalize()?
                || mp4.canonicalize()? == output.canonicalize()?))
    {
        return Err(Error::Invalid("motion delivery destination".into()));
    }
    if let Some(burst) = burst {
        burst.validate()?;
    }
    let burst_fields = burst
        .map(|b| {
            format!(
                " GCamera:BurstID=\"{}\" GCamera:BurstPrimary=\"{}\"",
                b.group_id,
                if b.primary { 1 } else { 0 }
            )
        })
        .unwrap_or_default();
    use photobridge_core::{Error, Result};
    use std::{
        fs::{File, OpenOptions},
        io::{Read, Seek, SeekFrom, Write},
    };
    let mut still = File::open(jpeg)?;
    let mut magic = [0u8; 2];
    still.read_exact(&mut magic)?;
    if magic != [0xff, 0xd8] {
        return Err(Error::Unsupported("motion JPEG input".into()));
    }
    // Do not create conflicting XMP packets; the native codec provides a clean
    // JPEG and preserves non-XMP EXIF before this packaging operation.
    loop {
        let mut marker = [0u8; 2];
        still.read_exact(&mut marker)?;
        if marker[0] != 0xff {
            return Err(Error::Integrity);
        }
        if marker[1] == 0xda {
            break;
        }
        if marker[1] == 0xd9 || marker[1] == 0 || marker[1] == 0xff {
            return Err(Error::Integrity);
        }
        let mut length = [0u8; 2];
        still.read_exact(&mut length)?;
        let length = u16::from_be_bytes(length) as usize;
        if length < 2 {
            return Err(Error::Integrity);
        }
        let mut segment = vec![0; length - 2];
        still.read_exact(&mut segment)?;
        if marker[1] == 0xe1 && segment.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
            return Err(Error::Unsupported("preexisting XMP packet".into()));
        }
    }
    let mut video = File::open(mp4)?;
    let video_len = video.metadata()?.len();
    let mut header = [0u8; 12];
    video.read_exact(&mut header)?;
    if &header[4..8] != b"ftyp" || video_len < 16 {
        return Err(Error::Unsupported("motion MP4 input".into()));
    }
    video.seek(SeekFrom::Start(0))?;
    let time = timestamp_us
        .map(|v| format!(" GCamera:MotionPhotoPresentationTimestampUs=\"{v}\""))
        .unwrap_or_default();
    let xmp = format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:GCamera="http://ns.google.com/photos/1.0/camera/" xmlns:Container="http://ns.google.com/photos/1.0/container/" xmlns:Item="http://ns.google.com/photos/1.0/container/item/" GCamera:MotionPhoto="1" GCamera:MotionPhotoVersion="1"{time}{burst_fields}><Container:Directory><rdf:Seq><rdf:li rdf:parseType="Resource"><Container:Item Item:Mime="image/jpeg" Item:Semantic="Primary" Item:Length="0" Item:Padding="0"/></rdf:li><rdf:li rdf:parseType="Resource"><Container:Item Item:Mime="video/mp4" Item:Semantic="MotionPhoto" Item:Length="{video_len}" Item:Padding="0"/></rdf:li></rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>"#
    );
    let mut packet = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    packet.extend(xmp.as_bytes());
    let size: u16 = (packet.len() + 2).try_into().map_err(|_| Error::Capacity)?;
    let partial = output.with_extension("motion.partial");
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let result = (|| -> Result<()> {
        out.write_all(&[0xff, 0xd8, 0xff, 0xe1])?;
        out.write_all(&size.to_be_bytes())?;
        out.write_all(&packet)?;
        still.seek(SeekFrom::Start(2))?;
        std::io::copy(&mut still, &mut out)?;
        std::io::copy(&mut video, &mut out)?;
        out.sync_all()?;
        drop(out);
        std::fs::rename(&partial, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}
