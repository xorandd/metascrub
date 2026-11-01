//! WebP: drop EXIF and XMP chunks (and unknown chunks), clear the matching
//! flags in the VP8X header and fix the RIFF size.

use anyhow::{Result, bail};

use crate::{Report, show, tiff};

const KEEP: &[&[u8; 4]] = &[b"VP8 ", b"VP8L", b"VP8X", b"ALPH", b"ANIM", b"ANMF", b"ICCP"];

pub fn clean(d: &[u8], rep: &mut Report) -> Result<Vec<u8>> {
    let riff_size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let end = (riff_size + 8).min(d.len());
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&d[..12]);
    let mut vp8x_at = None;
    let mut p = 12;

    while p + 8 <= end {
        let typ: [u8; 4] = [d[p], d[p + 1], d[p + 2], d[p + 3]];
        let len = u32::from_le_bytes([d[p + 4], d[p + 5], d[p + 6], d[p + 7]]) as usize;
        let padded = len + (len & 1);
        let Some(stop) = (p + 8).checked_add(padded).filter(|&s| s <= end) else {
            bail!("WebP is corrupt (chunk overruns file)");
        };
        let data = &d[p + 8..p + 8 + len];

        if KEEP.iter().any(|k| **k == typ) {
            if &typ == b"VP8X" {
                vp8x_at = Some(out.len() + 8);
            }
            out.extend_from_slice(&d[p..stop]);
        } else {
            match &typ {
                b"EXIF" => {
                    let t = data.strip_prefix(b"Exif\0\0").unwrap_or(data);
                    rep.add("EXIF", tiff::parse(t).map(|e| e.fields).unwrap_or_default());
                }
                b"XMP " => rep.add("XMP", vec![]),
                _ => rep.add("Other chunk", vec![("Chunk".into(), show(&typ))]),
            }
        }
        p = stop;
    }

    if let Some(at) = vp8x_at
        && let Some(flags) = out.get_mut(at)
    {
        *flags &= !(0x08 | 0x04); // EXIF and XMP present bits
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    if end < d.len() {
        rep.add("Hidden data after image", vec![("Size".into(), format!("{} bytes", d.len() - end))]);
    }
    Ok(out)
}
