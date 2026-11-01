//! Minimal read-only TIFF/EXIF parser, used only to *report* what a file
//! contained (camera, dates, GPS...) and to read the JPEG orientation flag.
//! Every read is bounds-checked; malformed data just yields fewer fields.

use crate::show;

struct R<'a> {
    d: &'a [u8],
    le: bool,
}

impl R<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b = self.d.get(o..o.checked_add(2)?)?;
        Some(if self.le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    }
    fn u32(&self, o: usize) -> Option<u32> {
        let b = self.d.get(o..o.checked_add(4)?)?;
        let a = [b[0], b[1], b[2], b[3]];
        Some(if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }
}

struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    /// Offset of the 4-byte value/offset field.
    val: usize,
}

fn type_size(typ: u16) -> usize {
    match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

fn entries(r: &R, off: usize) -> Vec<Entry> {
    let Some(n) = r.u16(off) else { return Vec::new() };
    // A real IFD never has thousands of entries; cap to avoid silly loops.
    (0..n.min(1000) as usize)
        .filter_map(|i| {
            let e = off + 2 + i * 12;
            Some(Entry { tag: r.u16(e)?, typ: r.u16(e + 2)?, count: r.u32(e + 4)?, val: e + 8 })
        })
        .collect()
}

/// Location of an entry's data (inline if it fits in 4 bytes).
fn data<'a>(r: &R<'a>, e: &Entry) -> Option<&'a [u8]> {
    let size = type_size(e.typ).checked_mul(e.count as usize)?;
    let start = if size <= 4 { e.val } else { r.u32(e.val)? as usize };
    r.d.get(start..start.checked_add(size)?)
}

fn ascii(r: &R, e: &Entry) -> Option<String> {
    if e.typ != 2 {
        return None;
    }
    let s = show(data(r, e)?);
    (!s.is_empty()).then_some(s)
}

fn rationals(r: &R, e: &Entry) -> Option<Vec<f64>> {
    if e.typ != 5 {
        return None;
    }
    let d = data(r, e)?;
    let sub = R { d, le: r.le };
    Some(
        (0..e.count as usize)
            .filter_map(|i| {
                let num = sub.u32(i * 8)? as f64;
                let den = sub.u32(i * 8 + 4)? as f64;
                (den != 0.0).then_some(num / den)
            })
            .collect(),
    )
}

pub struct Exif {
    pub fields: Vec<(String, String)>,
    pub orientation: Option<u16>,
}

const IFD0_TAGS: &[(u16, &str)] = &[
    (0x010E, "Description"),
    (0x010F, "Camera make"),
    (0x0110, "Camera model"),
    (0x0131, "Software"),
    (0x0132, "Modified"),
    (0x013B, "Artist"),
    (0x8298, "Copyright"),
];

const EXIF_TAGS: &[(u16, &str)] = &[
    (0x9003, "Taken"),
    (0xA430, "Camera owner"),
    (0xA431, "Camera serial"),
    (0xA434, "Lens"),
    (0xA435, "Lens serial"),
];

/// Parse a TIFF structure (the part after "Exif\0\0").
pub fn parse(d: &[u8]) -> Option<Exif> {
    let le = match d.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let r = R { d, le };
    if r.u16(2)? != 42 {
        return None;
    }
    let ifd0 = entries(&r, r.u32(4)? as usize);
    let mut fields = Vec::new();
    let mut orientation = None;

    let collect = |es: &[Entry], names: &[(u16, &str)], fields: &mut Vec<(String, String)>| {
        for (tag, name) in names {
            if let Some(v) = es.iter().find(|e| e.tag == *tag).and_then(|e| ascii(&r, e)) {
                fields.push((name.to_string(), v));
            }
        }
    };
    collect(&ifd0, IFD0_TAGS, &mut fields);

    for e in &ifd0 {
        match e.tag {
            0x0112 if e.typ == 3 => orientation = r.u16(e.val),
            0x8769 => {
                if let Some(off) = r.u32(e.val) {
                    collect(&entries(&r, off as usize), EXIF_TAGS, &mut fields);
                }
            }
            0x8825 => {
                if let Some(off) = r.u32(e.val) {
                    let gps = entries(&r, off as usize);
                    match gps_coords(&r, &gps) {
                        Some((lat, lon)) => fields.push(("GPS location".into(), format!("{lat:.5}, {lon:.5}"))),
                        None if !gps.is_empty() => fields.push(("GPS data".into(), "present".into())),
                        None => {}
                    }
                }
            }
            _ => {}
        }
    }
    Some(Exif { fields, orientation })
}

fn gps_coords(r: &R, gps: &[Entry]) -> Option<(f64, f64)> {
    let find = |t: u16| gps.iter().find(|e| e.tag == t);
    let dms = |t: u16| -> Option<f64> {
        let v = rationals(r, find(t)?)?;
        Some(v.first()? + v.get(1).unwrap_or(&0.0) / 60.0 + v.get(2).unwrap_or(&0.0) / 3600.0)
    };
    let mut lat = dms(2)?;
    let mut lon = dms(4)?;
    if find(1).and_then(|e| ascii(r, e)).is_some_and(|s| s.starts_with('S')) {
        lat = -lat;
    }
    if find(3).and_then(|e| ascii(r, e)).is_some_and(|s| s.starts_with('W')) {
        lon = -lon;
    }
    Some((lat, lon))
}

/// A minimal big-endian EXIF block holding only the orientation tag.
pub fn orientation_only(orientation: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(26);
    v.extend_from_slice(b"MM\0\x2A\0\0\0\x08"); // header, IFD0 at offset 8
    v.extend_from_slice(&1u16.to_be_bytes()); // one entry
    v.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
    v.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    v.extend_from_slice(&1u32.to_be_bytes()); // count
    v.extend_from_slice(&orientation.to_be_bytes());
    v.extend_from_slice(&[0, 0]); // padding
    v.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
    v
}
