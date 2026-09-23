//! Ultra HDR (JPEG_R) awareness for Motion Photo packaging.
//!
//! On Android 14+ `Bitmap.compress(JPEG)` keeps a decoded gain map and writes an
//! Ultra HDR JPEG: a primary XMP packet (`hdrgm:Version` plus a GContainer
//! directory listing Primary then GainMap) and an MPF APP2 segment whose second
//! entry locates the gain map JPEG stored directly after the primary image.
//! iPhone HDR photos decode with a gain map, so their prepared stills take this
//! form. Packaging must merge into that directory and keep MPF consistent
//! instead of refusing the preexisting XMP packet.
use photobridge_core::{Error, Result};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use std::ops::Range;

const CAMERA: &[u8] = b"http://ns.google.com/photos/1.0/camera/";
const CONTAINER: &[u8] = b"http://ns.google.com/photos/1.0/container/";
const ITEM: &[u8] = b"http://ns.google.com/photos/1.0/container/item/";
const HDRGM: &[u8] = b"http://ns.adobe.com/hdr-gain-map/1.0/";
const MP_ENTRY: u16 = 0xb002;

fn unsupported() -> Error {
    Error::Unsupported("Ultra HDR JPEG layout".into())
}

pub(crate) struct GainMap {
    pub version: String,
    pub length: u64,
}

/// Accepts only the directory an Ultra HDR encoder writes: Primary, then GainMap.
/// Anything else (camera attributes, other items, duplicate versions) is refused.
pub(crate) fn directory(xml: &[u8]) -> Result<GainMap> {
    let mut reader = NsReader::from_reader(xml);
    let mut version = None;
    let mut items = Vec::new();
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|_| unsupported())?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                if matches!(&event, Event::Start(_)) {
                    depth += 1;
                }
                if depth > 128 {
                    return Err(unsupported());
                }
                let (mut semantic, mut mime, mut length) = (None, None, None);
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| unsupported())?;
                    let (ns, name) = reader.resolver().resolve_attribute(attr.key);
                    let ResolveResult::Bound(ns) = ns else {
                        continue;
                    };
                    match (ns.as_ref(), name.as_ref()) {
                        (CAMERA, _) => return Err(unsupported()),
                        (HDRGM, b"Version") => {
                            let value = std::str::from_utf8(&attr.value)
                                .ok()
                                .filter(|v| {
                                    !v.is_empty()
                                        && v.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                                })
                                .ok_or_else(unsupported)?;
                            if version.replace(value.to_owned()).is_some() {
                                return Err(unsupported());
                            }
                        }
                        (ITEM, b"Semantic") => semantic = Some(attr.value.to_vec()),
                        (ITEM, b"Mime") => mime = Some(attr.value.to_vec()),
                        (ITEM, b"Length") => {
                            length = Some(
                                std::str::from_utf8(&attr.value)
                                    .ok()
                                    .and_then(|v| v.parse::<u64>().ok())
                                    .ok_or_else(unsupported)?,
                            )
                        }
                        _ => {}
                    }
                }
                let (ns, name) = reader.resolver().resolve_element(e.name());
                if matches!(ns, ResolveResult::Bound(n) if n.as_ref() == CONTAINER)
                    && name.as_ref() == b"Item"
                {
                    items.push((semantic, mime, length));
                }
            }
            Event::End(_) => depth = depth.checked_sub(1).ok_or_else(unsupported)?,
            Event::DocType(_) => return Err(unsupported()),
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 {
        return Err(unsupported());
    }
    let jpeg = Some(b"image/jpeg".to_vec());
    match (version, items.as_slice()) {
        (Some(version), [(Some(primary), pm, _), (Some(gain), gm, Some(length))])
            if primary == b"Primary"
                && gain == b"GainMap"
                && *pm == jpeg
                && *gm == jpeg
                && *length > 0 =>
        {
            Ok(GainMap {
                version,
                length: *length,
            })
        }
        _ => Err(unsupported()),
    }
}

pub(crate) struct Entry {
    pub size: u32,
    pub offset: u32,
}

/// A parsed MPF APP2 segment. `header` is the absolute position of the TIFF
/// header that MP entry offsets are relative to.
pub(crate) struct Mpf {
    pub header: usize,
    big_endian: bool,
    entries: usize,
    pub count: usize,
}

impl Mpf {
    /// `payload` is the APP2 payload (after the length field), starting with `MPF\0`.
    pub fn parse(bytes: &[u8], payload: Range<usize>) -> Result<Self> {
        let invalid = || Error::Unsupported("multi-picture JPEG".into());
        let header = payload.start + 4;
        let big_endian = match bytes.get(header..header + 4) {
            Some(b"MM\0*") => true,
            Some(b"II*\0") => false,
            _ => return Err(invalid()),
        };
        let mpf = Self {
            header,
            big_endian,
            entries: 0,
            count: 0,
        };
        let within = |start: usize, len: usize| {
            start
                .checked_add(len)
                .filter(|end| start >= header && *end <= payload.end)
                .ok_or_else(invalid)
        };
        within(header, 8)?;
        let ifd = header + mpf.u32(bytes, header + 4) as usize;
        within(ifd, 2)?;
        let fields = mpf.u16(bytes, ifd) as usize;
        within(ifd + 2, fields * 12)?;
        let (mut entries, mut count) = (None, 0);
        for field in 0..fields {
            let at = ifd + 2 + field * 12;
            if mpf.u16(bytes, at) == MP_ENTRY {
                let len = mpf.u32(bytes, at + 4) as usize;
                if mpf.u16(bytes, at + 2) != 7
                    || len == 0
                    || !len.is_multiple_of(16)
                    || entries.is_some()
                {
                    return Err(invalid());
                }
                let start = header + mpf.u32(bytes, at + 8) as usize;
                within(start, len)?;
                entries = Some(start);
                count = len / 16;
            }
        }
        let mpf = Self {
            entries: entries.ok_or_else(invalid)?,
            count,
            ..mpf
        };
        let primary = mpf.entry(bytes, 0);
        if primary.offset != 0 || primary.size as usize > bytes.len() {
            return Err(invalid());
        }
        for index in 1..count {
            let entry = mpf.entry(bytes, index);
            let start = header + entry.offset as usize;
            if entry.offset == 0 || start + entry.size as usize > bytes.len() {
                return Err(invalid());
            }
        }
        Ok(mpf)
    }

    pub fn entry(&self, bytes: &[u8], index: usize) -> Entry {
        let at = self.entries + index * 16;
        Entry {
            size: self.u32(bytes, at + 4),
            offset: self.u32(bytes, at + 8),
        }
    }

    /// Rewrites entry geometry in place after the bytes before the secondary
    /// images changed length: the primary image entry becomes `primary_size`,
    /// and secondary offsets (relative to the MP header) move by `offset_delta`.
    pub fn rebase(&self, bytes: &mut [u8], primary_size: u32, offset_delta: i64) -> Result<()> {
        self.put_u32(bytes, self.entries + 4, primary_size);
        for index in 1..self.count {
            let offset = u32::try_from(i64::from(self.entry(bytes, index).offset) + offset_delta)
                .map_err(|_| Error::Capacity)?;
            self.put_u32(bytes, self.entries + index * 16 + 8, offset);
        }
        Ok(())
    }

    fn u16(&self, bytes: &[u8], at: usize) -> u16 {
        let raw = [bytes[at], bytes[at + 1]];
        if self.big_endian {
            u16::from_be_bytes(raw)
        } else {
            u16::from_le_bytes(raw)
        }
    }

    fn u32(&self, bytes: &[u8], at: usize) -> u32 {
        let raw = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if self.big_endian {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        }
    }

    fn put_u32(&self, bytes: &mut [u8], at: usize, value: u32) {
        let raw = if self.big_endian {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        bytes[at..at + 4].copy_from_slice(&raw);
    }
}
