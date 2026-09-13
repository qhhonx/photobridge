//! Lossless JPEG annotation for derived burst delivery copies.
use photobridge_core::{BurstMetadata, Error, Result};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
const XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const RDF: &[u8] = b"http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const CAMERA: &[u8] = b"http://ns.google.com/photos/1.0/camera/";
fn invalid() -> Error {
    Error::Unsupported("burst JPEG metadata".into())
}
fn description(burst: &BurstMetadata) -> String {
    format!(
        r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" rdf:about="" xmlns:GCamera="http://ns.google.com/photos/1.0/camera/" GCamera:BurstID="{}" GCamera:BurstPrimary="{}"/>"#,
        burst.group_id,
        if burst.primary { 1 } else { 0 }
    )
}
fn augment(xml: Option<&[u8]>, burst: &BurstMetadata) -> Result<Vec<u8>> {
    let desc = description(burst);
    let Some(xml) = xml else {
        return Ok(format!(r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">{desc}</rdf:RDF></x:xmpmeta>"#).into_bytes());
    };
    let mut reader = NsReader::from_reader(xml);
    let mut insertion = None;
    let mut rdf_count = 0;
    let mut existing_id = false;
    let mut existing_primary = false;
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|_| invalid())?;
        let end = reader.buffer_position() as usize;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                if matches!(&event, Event::Start(_)) {
                    depth += 1;
                }
                if depth > 128 {
                    return Err(invalid());
                }
                let (ns, name) = reader.resolver().resolve_element(e.name());
                if matches!(ns,ResolveResult::Bound(n) if n.as_ref()==CAMERA)
                    && [b"BurstID".as_slice(), b"BurstPrimary".as_slice()].contains(&name.as_ref())
                {
                    return Err(invalid());
                }
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| invalid())?;
                    let (ns, name) = reader.resolver().resolve_attribute(attr.key);
                    if matches!(ns,ResolveResult::Bound(n) if n.as_ref()==CAMERA) {
                        match name.as_ref() {
                            b"BurstID" => {
                                if attr.value.as_ref() != burst.group_id.as_bytes() {
                                    return Err(invalid());
                                }
                                existing_id = true;
                            }
                            b"BurstPrimary" => {
                                if attr.value.as_ref() != if burst.primary { b"1" } else { b"0" } {
                                    return Err(invalid());
                                }
                                existing_primary = true;
                            }
                            _ => {}
                        }
                    }
                }
                let (ns, name) = reader.resolver().resolve_element(e.name());
                if matches!(ns,ResolveResult::Bound(n) if n.as_ref()==RDF)
                    && name.as_ref() == b"RDF"
                {
                    rdf_count += 1;
                    if matches!(&event, Event::Empty(_)) {
                        let mut replacement = xml[start..end - 2].to_vec();
                        replacement.push(b'>');
                        replacement.extend(desc.as_bytes());
                        replacement.extend(b"</");
                        replacement.extend(e.name().as_ref());
                        replacement.push(b'>');
                        insertion = Some((start, end, replacement));
                    }
                }
            }
            Event::End(e) => {
                depth = depth.checked_sub(1).ok_or_else(invalid)?;
                let (ns, name) = reader.resolver().resolve_element(e.name());
                if matches!(ns,ResolveResult::Bound(n) if n.as_ref()==RDF)
                    && name.as_ref() == b"RDF"
                {
                    insertion = Some((start, start, desc.as_bytes().to_vec()));
                }
            }
            Event::DocType(_) => return Err(invalid()),
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || rdf_count != 1 || existing_id != existing_primary {
        return Err(invalid());
    }
    if existing_id {
        return Ok(xml.to_vec());
    }
    let (start, end, replacement) = insertion.ok_or_else(invalid)?;
    let mut output = Vec::with_capacity(xml.len() + replacement.len());
    output.extend(&xml[..start]);
    output.extend(replacement);
    output.extend(&xml[end..]);
    Ok(output)
}

pub fn write_jpeg_burst(source: &Path, output: &Path, burst: &BurstMetadata) -> Result<()> {
    burst.validate()?;
    if source == output || (output.exists() && source.canonicalize()? == output.canonicalize()?) {
        return Err(Error::Invalid("burst delivery destination".into()));
    }
    let mut input = File::open(source)?;
    let mut magic = [0; 2];
    input.read_exact(&mut magic)?;
    if magic != [0xff, 0xd8] {
        return Err(invalid());
    }
    let mut old = None;
    let mut xml = None;
    let mut found_scan = false;
    for _ in 0..4096 {
        let start = input.stream_position()?;
        let mut marker = [0; 2];
        input.read_exact(&mut marker)?;
        if marker[0] != 0xff {
            return Err(invalid());
        }
        while marker[1] == 0xff {
            input.read_exact(&mut marker[1..])?;
        }
        if marker[1] == 0xda {
            found_scan = true;
            break;
        }
        if marker[1] == 0xd9
            || marker[1] == 0
            || marker[1] == 1
            || (0xd0..=0xd8).contains(&marker[1])
        {
            return Err(invalid());
        }
        let mut length = [0; 2];
        input.read_exact(&mut length)?;
        let length = u16::from_be_bytes(length) as usize;
        if length < 2 {
            return Err(invalid());
        }
        let mut segment = vec![0; length - 2];
        input.read_exact(&mut segment)?;
        // Multi-picture JPEG offsets need a dedicated rewriter; do not silently
        // invalidate an HDR gain map or a secondary image in a delivery copy.
        if marker[1] == 0xe2 && segment.starts_with(b"MPF\0") {
            return Err(invalid());
        }
        if marker[1] == 0xe1 && segment.starts_with(b"http://ns.adobe.com/xmp/extension/\0") {
            return Err(invalid());
        }
        if marker[1] == 0xe1 && segment.starts_with(XMP) {
            if old.is_some() {
                return Err(invalid());
            }
            old = Some((start, input.stream_position()?));
            xml = Some(segment[XMP.len()..].to_vec());
        }
    }
    if !found_scan {
        return Err(invalid());
    }
    let mut packet = XMP.to_vec();
    packet.extend(augment(xml.as_deref(), burst)?);
    let length: u16 = (packet.len() + 2).try_into().map_err(|_| Error::Capacity)?;
    let partial = output.with_extension("burst.partial");
    // Never remove another operation's partial on a create_new failure.
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let result = (|| -> Result<()> {
        out.write_all(&[0xff, 0xd8, 0xff, 0xe1])?;
        out.write_all(&length.to_be_bytes())?;
        out.write_all(&packet)?;
        input.seek(SeekFrom::Start(2))?;
        if let Some((start, end)) = old {
            let copied = std::io::copy(&mut (&mut input).take(start - 2), &mut out)?;
            if copied != start - 2 {
                return Err(Error::Integrity);
            }
            input.seek(SeekFrom::Start(end))?;
        }
        std::io::copy(&mut input, &mut out)?;
        out.sync_all()?;
        std::fs::rename(&partial, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(partial);
    }
    result
}
