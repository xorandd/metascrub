//! JPEG: keep only the segments needed to display the image correctly.
//!
//! Kept: JFIF (APP0), ICC colour profile (APP2), Adobe colour info (APP14) and
//! all image-data segments. Removed: EXIF, XMP, IPTC/Photoshop, comments, other
//! APPn blocks, and any data hidden after the end-of-image marker.

use anyhow::{Result, bail};

use crate::{Options, Report, show, tiff};

pub fn clean(d: &[u8], opts: &Options, rep: &mut Report) -> Result<Vec<u8>> {
    if !d.starts_with(&[0xFF, 0xD8]) {
        bail!("not a JPEG file");
    }
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    let mut p = 2;
    let mut orientation: Option<u16> = None;
    let mut exif_pos: Option<usize> = None;
    let mut end = None;

    while end.is_none() {
        if p >= d.len() {
            bail!("JPEG is truncated (no image data found)");
        }
        if d[p] != 0xFF {
            bail!("JPEG is corrupt (bad marker at byte {p})");
        }
        while p < d.len() && d[p] == 0xFF {
            p += 1; // fill bytes
        }
        let Some(&m) = d.get(p) else { bail!("JPEG is truncated") };
        p += 1;

        match m {
            0xD9 => {
                out.extend_from_slice(&[0xFF, 0xD9]);
                end = Some(p);
                continue;
            }
            0x01 | 0xD0..=0xD7 => {
                out.extend_from_slice(&[0xFF, m]);
                continue;
            }
            _ => {}
        }

        let Some(len) = d.get(p..p + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize) else {
            bail!("JPEG is truncated");
        };
        if len < 2 || p + len > d.len() {
            bail!("JPEG is corrupt (segment overruns file)");
        }
        let payload = &d[p + 2..p + len];

        if m == 0xDA {
            // Start of scan: copy compressed data through to the end-of-image marker.
            // Inside scan data a real 0xFF byte is always followed by 0x00 or a
            // restart marker, so the first FF D9 is the true end.
            let mut i = p + len;
            let mut eoi = None;
            while i + 1 < d.len() {
                if d[i] == 0xFF && d[i + 1] == 0xD9 {
                    eoi = Some(i + 2);
                    break;
                }
                i += 1;
            }
            out.extend_from_slice(&[0xFF, 0xDA]);
            match eoi {
                Some(e) => {
                    out.extend_from_slice(&d[p..e]);
                    end = Some(e);
                }
                None => {
                    out.extend_from_slice(&d[p..]);
                    rep.notes.push("image has no end marker (file may be truncated)".into());
                    end = Some(d.len());
                }
            }
            continue;
        }

        let drop: Option<(&str, Vec<(String, String)>)> = match m {
            0xE0 if payload.starts_with(b"JFIF\0") || payload.starts_with(b"JFXX\0") => None,
            0xE0 => Some(("APP0 block", vec![])),
            0xE1 if payload.starts_with(b"Exif\0\0") => {
                let mut fields = vec![];
                if let Some(ex) = tiff::parse(&payload[6..]) {
                    orientation = orientation.or(ex.orientation);
                    fields = ex.fields;
                }
                if exif_pos.is_none() {
                    exif_pos = Some(out.len());
                }
                // A tiny EXIF block holding only the rotation flag is what we
                // write ourselves; replacing it isn't worth reporting.
                let only_rotation = fields.is_empty() && payload.len() <= 40 && opts.keep_orientation;
                Some((if only_rotation { "" } else { "EXIF" }, fields))
            }
            0xE1 if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") => Some(("XMP", vec![])),
            0xE1 if payload.starts_with(b"http://ns.adobe.com/xmp/extension/") => Some(("Extended XMP", vec![])),
            0xE1 => Some(("APP1 block", vec![])),
            0xE2 if payload.starts_with(b"ICC_PROFILE\0") => None,
            0xE2 if payload.starts_with(b"MPF\0") => Some(("Multi-picture info", vec![])),
            0xE2 => Some(("APP2 block", vec![])),
            0xED => Some(("IPTC / Photoshop", vec![])),
            0xEE => None, // Adobe colour transform, needed for CMYK files
            0xE3..=0xEF => Some(("Vendor APP block", vec![])),
            0xFE => Some(("Comment", vec![("Text".into(), show(payload))])),
            _ => None,
        };

        match drop {
            Some(("", _)) => {}
            Some((label, fields)) => merge(rep, label, fields),
            None => {
                out.extend_from_slice(&[0xFF, m]);
                out.extend_from_slice(&d[p..p + len]);
            }
        }
        p += len;
    }

    let end = end.unwrap_or(d.len());
    if end < d.len() {
        rep.add("Hidden data after image", vec![("Size".into(), format!("{} bytes", d.len() - end))]);
    }

    if opts.keep_orientation
        && let (Some(o @ 2..=8), Some(pos)) = (orientation, exif_pos)
    {
        let tiff = tiff::orientation_only(o);
        let mut seg = vec![0xFF, 0xE1];
        seg.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        seg.extend_from_slice(b"Exif\0\0");
        seg.extend_from_slice(&tiff);
        out.splice(pos..pos, seg);
        if rep.removed.iter().any(|i| i.label == "EXIF") {
            rep.notes.push("the rotation flag is kept so the photo isn't shown sideways".into());
        }
    }
    Ok(out)
}

/// Combine repeated blocks of the same kind into one report line.
fn merge(rep: &mut Report, label: &str, fields: Vec<(String, String)>) {
    match rep.removed.iter_mut().find(|i| i.label == label) {
        Some(item) => item.fields.extend(fields),
        None => rep.add(label, fields),
    }
}
