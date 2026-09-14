//! Date metadata belongs to the delivery copy, never to the received original.
use little_exif::{exif_tag::ExifTag, metadata::Metadata};
use photobridge_core::{Error, Result};
use std::{fs, path::Path};

pub fn write_photo_date(source: &Path, output: &Path, date: &str, subsecond: u16) -> Result<()> {
    let valid_date = date.len() == 19
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 | 13 | 16 => b == b':',
            10 => b == b' ',
            _ => b.is_ascii_digit(),
        });
    if subsecond > 999 || !valid_date || source == output || output.exists() {
        return Err(Error::Invalid("photo date destination".into()));
    }
    if !matches!(
        output.extension().and_then(|s| s.to_str()),
        Some("heic" | "heif" | "jpg" | "png")
    ) || source.metadata()?.len() > 64 << 20
    {
        return Err(Error::Unsupported("photo date metadata".into()));
    }
    // The caller only requests this when the original capture date is absent.
    // Reading and writing containers does not decode/re-encode image pixels.
    fs::copy(source, output)?;
    let result = (|| {
        let mut metadata = match Metadata::new_from_path(output) {
            Ok(value) => value,
            // little_exif 0.6.23 distinguishes absent metadata from malformed
            // metadata with these exact errors. Never discard a parse failure.
            Err(error)
                if matches!(
                    error.to_string().as_str(),
                    "No EXIF item found!" | "No EXIF data found!" | "No metadata found!"
                ) =>
            {
                Metadata::new()
            }
            Err(error) => return Err(error.into()),
        };
        metadata.set_tag(ExifTag::DateTimeOriginal(date.into()));
        metadata.set_tag(ExifTag::SubSecTimeOriginal(format!("{subsecond:03}")));
        metadata.set_tag(ExifTag::SubSecTimeDigitized(format!("{subsecond:03}")));
        metadata.set_tag(ExifTag::OffsetTimeOriginal("+00:00".into()));
        metadata.set_tag(ExifTag::CreateDate(date.into()));
        metadata.set_tag(ExifTag::OffsetTimeDigitized("+00:00".into()));
        metadata.write_to_file(output)?;
        let reread = Metadata::new_from_path(output)?;
        if !matches!(reread.get_tag(&ExifTag::DateTimeOriginal(String::new())).next(),
            Some(ExifTag::DateTimeOriginal(value)) if value == date)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}
