//! Lossless HEIC + QuickTime Live Photo packaging for the Pixel receiver.
//! Only the HEIF item tables and XMP are rewritten; image and video payloads
//! are copied byte-for-byte. Unsupported HEIF layouts use the codec fallback.
use photobridge_core::{BurstMetadata, Error, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

fn unsupported() -> Error {
    Error::Unsupported("HEIC motion container layout".into())
}
fn number(data: &[u8], at: usize, width: usize) -> Result<u64> {
    if width > 8 {
        return Err(unsupported());
    }
    let bytes = data
        .get(at..at.checked_add(width).ok_or_else(unsupported)?)
        .ok_or_else(unsupported)?;
    Ok(bytes.iter().fold(0u64, |n, b| n << 8 | u64::from(*b)))
}
fn put_number(data: &mut [u8], at: usize, width: usize, value: u64) -> Result<()> {
    if width > 8 || (width < 8 && value >= 1u64 << (width * 8)) {
        return Err(unsupported());
    }
    let target = data
        .get_mut(at..at.checked_add(width).ok_or_else(unsupported)?)
        .ok_or_else(unsupported)?;
    for (i, byte) in target.iter_mut().enumerate() {
        *byte = (value >> ((width - i - 1) * 8)) as u8;
    }
    Ok(())
}
fn boxes(data: &[u8], start: usize, end: usize) -> Result<Vec<(usize, usize, [u8; 4])>> {
    let mut result = Vec::new();
    let mut at = start;
    while at < end {
        let short_size = number(data, at, 4)?;
        let size = usize::try_from(if short_size == 1 {
            number(data, at + 8, 8)?
        } else {
            short_size
        })
        .map_err(|_| unsupported())?;
        let typ: [u8; 4] = data
            .get(at + 4..at + 8)
            .ok_or_else(unsupported)?
            .try_into()
            .unwrap();
        if size < (if short_size == 1 { 16 } else { 8 })
            || at.checked_add(size).filter(|v| *v <= end).is_none()
        {
            return Err(unsupported());
        }
        result.push((at, size, typ));
        at += size;
    }
    Ok(result)
}
fn box_data(typ: &[u8; 4], payload: &[u8]) -> Result<Vec<u8>> {
    let size = u32::try_from(payload.len().checked_add(8).ok_or(Error::Capacity)?)
        .map_err(|_| Error::Capacity)?;
    let mut result = Vec::with_capacity(size as usize);
    result.extend(size.to_be_bytes());
    result.extend(typ);
    result.extend(payload);
    Ok(result)
}
fn extend_count(
    mut value: Vec<u8>,
    count_at: usize,
    width: usize,
    extra: &[u8],
) -> Result<Vec<u8>> {
    let count = number(&value, count_at, width)?;
    put_number(
        &mut value,
        count_at,
        width,
        count.checked_add(1).ok_or(Error::Capacity)?,
    )?;
    value.extend(extra);
    let size = u32::try_from(value.len()).map_err(|_| Error::Capacity)?;
    value[..4].copy_from_slice(&size.to_be_bytes());
    Ok(value)
}

/// Append the unmodified MOV to a HEIC and register a Motion Photo XMP item.
pub fn write_heic_motion_with_burst(
    heic: &Path,
    mov: &Path,
    output: &Path,
    burst: Option<&BurstMetadata>,
) -> Result<()> {
    if heic == output || mov == output || output.exists() {
        return Err(Error::Invalid("motion delivery destination".into()));
    }
    if let Some(b) = burst {
        b.validate()?;
    }
    let still = fs::read(heic)?;
    if still.len() > 128 << 20 {
        return Err(unsupported());
    }
    if still
        .windows(b"GCamera:MotionPhoto=\"1\"".len())
        .any(|window| window == b"GCamera:MotionPhoto=\"1\"")
    {
        return Err(unsupported());
    }
    let top = boxes(&still, 0, still.len())?;
    if top.len() != 3 || top[0].2 != *b"ftyp" || top[1].2 != *b"meta" || top[2].2 != *b"mdat" {
        return Err(unsupported());
    }
    if !still[8..top[0].1]
        .windows(4)
        .any(|b| matches!(b, b"heic" | b"heix" | b"mif1"))
    {
        return Err(unsupported());
    }
    let mut video = File::open(mov)?;
    let video_size = video.metadata()?.len();
    if video_size < 16 || video_size > u32::MAX as u64 - 256 {
        return Err(unsupported());
    }
    let mut video_head = [0; 8];
    video.read_exact(&mut video_head)?;
    if &video_head[4..] != b"ftyp" {
        return Err(unsupported());
    }
    let meta = &still[top[1].0..top[1].0 + top[1].1];
    let children = boxes(meta, 12, meta.len())?;
    let child = |typ: &[u8; 4]| -> Result<&[u8]> {
        let matches: Vec<_> = children.iter().filter(|c| &c.2 == typ).collect();
        if matches.len() != 1 {
            return Err(unsupported());
        }
        let c = matches[0];
        Ok(&meta[c.0..c.0 + c.1])
    };
    let pitm = child(b"pitm")?;
    if pitm[8] != 0 {
        return Err(unsupported());
    }
    let primary_id = u16::try_from(number(pitm, 12, 2)?).map_err(|_| unsupported())?;
    let iinf = child(b"iinf")?;
    let iref = child(b"iref")?;
    let iloc = child(b"iloc")?;
    if iinf[8] != 0 || iref[8] != 0 || !matches!(iloc[8], 0 | 1) {
        return Err(unsupported());
    }
    let offset_size = (iloc[12] >> 4) as usize;
    let length_size = (iloc[12] & 15) as usize;
    let base_size = (iloc[13] >> 4) as usize;
    let index_size = if iloc[8] == 1 {
        (iloc[13] & 15) as usize
    } else {
        0
    };
    if !matches!(offset_size, 4 | 8)
        || !matches!(length_size, 4 | 8)
        || base_size != 0
        || index_size != 0
    {
        return Err(unsupported());
    }
    let count = number(iloc, 14, 2)? as usize;
    let mut at = 16;
    let mut offsets = Vec::new();
    let mut max_id = primary_id;
    let entries = boxes(iinf, 14, iinf.len())?;
    if entries.len() != number(iinf, 12, 2)? as usize {
        return Err(unsupported());
    }
    for (entry_at, _, kind) in entries {
        if kind != *b"infe" || iinf[entry_at + 8] != 2 {
            return Err(unsupported());
        }
        max_id = max_id.max(number(iinf, entry_at + 12, 2)? as u16);
    }
    for _ in 0..count {
        let id = number(iloc, at, 2)? as u16;
        max_id = max_id.max(id);
        at += 2;
        let method = if iloc[8] == 1 {
            let v = number(iloc, at, 2)?;
            at += 2;
            v & 15
        } else {
            0
        };
        let reference = number(iloc, at, 2)?;
        at += 2;
        if reference != 0 || method > 1 {
            return Err(unsupported());
        }
        let extent_count = number(iloc, at, 2)? as usize;
        at += 2;
        for _ in 0..extent_count {
            let offset = number(iloc, at, offset_size)?;
            if method == 0 {
                offsets.push((at, offset));
            }
            at += offset_size + length_size;
            if at > iloc.len() {
                return Err(unsupported());
            }
        }
    }
    if at != iloc.len() {
        return Err(unsupported());
    }
    let xmp_id = max_id.checked_add(1).ok_or_else(unsupported)?;
    let footer_size = samsung_footer(0, 0).len() as u64;
    // mpvd header and SEF footer are included in the MotionPhoto item length,
    // while the primary item declares the eight-byte mpvd header as padding.
    let motion_length = video_size + footer_size;
    let burst_fields = burst
        .map(|b| {
            format!(
                " GCamera:BurstID=\"{}\" GCamera:BurstPrimary=\"{}\"",
                b.group_id,
                u8::from(b.primary)
            )
        })
        .unwrap_or_default();
    let xmp = format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:GCamera="http://ns.google.com/photos/1.0/camera/" xmlns:Container="http://ns.google.com/photos/1.0/container/" xmlns:Item="http://ns.google.com/photos/1.0/container/item/" GCamera:MotionPhoto="1" GCamera:MotionPhotoVersion="1" GCamera:MotionPhotoPresentationTimestampUs="-1"{burst_fields}><Container:Directory><rdf:Seq><rdf:li rdf:parseType="Resource"><Container:Item Item:Mime="image/heic" Item:Semantic="Primary" Item:Length="0" Item:Padding="8"/></rdf:li><rdf:li rdf:parseType="Resource"><Container:Item Item:Mime="video/quicktime" Item:Semantic="MotionPhoto" Item:Length="{motion_length}" Item:Padding="0"/></rdf:li></rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>"#
    );
    let xmp = xmp.as_bytes();
    let mut infe = vec![2, 0, 0, 1];
    infe.extend(xmp_id.to_be_bytes());
    infe.extend([0, 0]);
    infe.extend(b"mime\0application/rdf+xml\0");
    let iinf_new = extend_count(iinf.to_vec(), 12, 2, &box_data(b"infe", &infe)?)?;
    let mut cdsc = Vec::new();
    cdsc.extend(xmp_id.to_be_bytes());
    cdsc.extend(1u16.to_be_bytes());
    cdsc.extend(primary_id.to_be_bytes());
    let mut iref_new = iref.to_vec();
    iref_new.extend(box_data(b"cdsc", &cdsc)?);
    let iref_size = u32::try_from(iref_new.len()).map_err(|_| Error::Capacity)?;
    iref_new[..4].copy_from_slice(&iref_size.to_be_bytes());
    let mut new_extent = Vec::new();
    new_extent.extend(xmp_id.to_be_bytes());
    if iloc[8] == 1 {
        new_extent.extend([0, 0]);
    }
    new_extent.extend([0, 0, 0, 1]);
    new_extent.extend(vec![0; offset_size]);
    new_extent.extend(vec![0; length_size]);
    let mut iloc_new = extend_count(iloc.to_vec(), 14, 2, &new_extent)?;
    let meta_delta =
        iinf_new.len() + iref_new.len() + iloc_new.len() - iinf.len() - iref.len() - iloc.len();
    let mdat_header = if number(&still, top[2].0, 4)? == 1 {
        16
    } else {
        8
    };
    let old_payload = top[2].0 + mdat_header;
    let new_payload = old_payload.checked_add(meta_delta).ok_or(Error::Capacity)?;
    for (position, old) in offsets {
        if old < old_payload as u64 || old >= still.len() as u64 {
            return Err(unsupported());
        }
        put_number(
            &mut iloc_new,
            position,
            offset_size,
            old + meta_delta as u64 + xmp.len() as u64,
        )?;
    }
    let new_entry_offset = iloc_new.len() - length_size - offset_size;
    put_number(
        &mut iloc_new,
        new_entry_offset,
        offset_size,
        new_payload as u64,
    )?;
    put_number(
        &mut iloc_new,
        new_entry_offset + offset_size,
        length_size,
        xmp.len() as u64,
    )?;
    let mut new_meta = meta[..12].to_vec();
    for c in children {
        new_meta.extend(match &c.2 {
            b"iinf" => &iinf_new,
            b"iref" => &iref_new,
            b"iloc" => &iloc_new,
            _ => &meta[c.0..c.0 + c.1],
        });
    }
    let meta_size = u32::try_from(new_meta.len()).map_err(|_| Error::Capacity)?;
    new_meta[..4].copy_from_slice(&meta_size.to_be_bytes());
    let mut mdat = still[top[2].0..old_payload].to_vec();
    let mdat_size = top[2].1.checked_add(xmp.len()).ok_or(Error::Capacity)?;
    if mdat_header == 16 {
        mdat[8..16].copy_from_slice(&(mdat_size as u64).to_be_bytes());
    } else {
        mdat[..4].copy_from_slice(
            &u32::try_from(mdat_size)
                .map_err(|_| Error::Capacity)?
                .to_be_bytes(),
        );
    }
    let image_size = top[0].1 + new_meta.len() + mdat_size;
    if image_size
        .checked_add(8)
        .filter(|size| *size <= u32::MAX as usize)
        .is_none()
    {
        return Err(unsupported());
    }
    let footer = samsung_footer(image_size, video_size as u32);
    let mpvd_size =
        u32::try_from(8 + video_size as usize + footer.len()).map_err(|_| Error::Capacity)?;
    let partial = output.with_extension("motion.partial");
    let mut out = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial)?;
    let result = (|| -> Result<()> {
        out.write_all(&still[..top[0].1])?;
        out.write_all(&new_meta)?;
        out.write_all(&mdat)?;
        out.write_all(xmp)?;
        out.write_all(&still[old_payload..])?;
        out.write_all(&mpvd_size.to_be_bytes())?;
        out.write_all(b"mpvd")?;
        let mut video = File::open(mov)?;
        std::io::copy(&mut video, &mut out)?;
        out.write_all(&footer)?;
        out.sync_all()?;
        drop(out);
        fs::rename(&partial, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn samsung_footer(image_size: usize, video_size: u32) -> Vec<u8> {
    let tags: [([u8; 4], &str, Vec<u8>); 2] = [
        ([0, 0, 0x30, 0x0a], "MotionPhoto_Data", {
            let mut v = b"mpv2".to_vec();
            v.extend(((image_size + 8) as u32).to_be_bytes());
            v.extend(video_size.to_be_bytes());
            v
        }),
        ([0, 0, 0x31, 0x0a], "MotionPhoto_Version", b"mpv3".to_vec()),
    ];
    let mut data = Vec::new();
    let mut lengths = Vec::new();
    for (id, name, value) in &tags {
        let start = data.len();
        data.extend(id);
        data.extend((name.len() as u32).to_le_bytes());
        data.extend(name.as_bytes());
        data.extend(value);
        lengths.push((data.len() - start) as u32);
    }
    let mut sefh = b"SEFH".to_vec();
    sefh.extend(107u32.to_le_bytes());
    sefh.extend(2u32.to_le_bytes());
    for (i, (id, _, _)) in tags.iter().enumerate() {
        sefh.extend(id);
        sefh.extend(lengths[..=i].iter().sum::<u32>().to_le_bytes());
        sefh.extend(lengths[i].to_le_bytes());
    }
    let sefh_len = sefh.len() as u32;
    sefh.extend(sefh_len.to_le_bytes());
    sefh.extend(b"SEFT");
    let mut result = Vec::new();
    result.extend(((8 + data.len() + sefh.len()) as u32).to_be_bytes());
    result.extend(b"sefd");
    result.extend(data);
    result.extend(sefh);
    result
}
