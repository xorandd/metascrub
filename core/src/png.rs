//! PNG: keep only chunks needed to render the image (including colour and
//! animation chunks); drop text, EXIF, timestamps and unknown ancillary chunks.

use anyhow::{Result, bail};

use crate::{Report, show, tiff};

const KEEP: &[&[u8; 4]] = &[
    b"IHDR", b"PLTE", b"IDAT", b"IEND", // critical
    b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"cICP", b"mDCv", b"cLLi", // colour
    b"pHYs", b"bKGD", b"hIST", // display
    b"acTL", b"fcTL", b"fdAT", // APNG animation
];

pub fn clean(d: &[u8], rep: &mut Report) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&d[..8]);
    let mut p = 8;
    let mut seen_end = false;

    while p < d.len() {
        let Some(hdr) = d.get(p..p + 8) else { bail!("PNG is truncated") };
        let len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let typ: [u8; 4] = [hdr[4], hdr[5], hdr[6], hdr[7]];
        let total = len.checked_add(12).filter(|t| p + t <= d.len());
        let Some(total) = total else { bail!("PNG is corrupt (chunk overruns file)") };
        let data = &d[p + 8..p + 8 + len];

        if KEEP.iter().any(|k| **k == typ) {
            out.extend_from_slice(&d[p..p + total]);
        } else if typ[0].is_ascii_uppercase() {
            // An unknown *critical* chunk: removing it could break the image.
            bail!("PNG has an unknown required chunk '{}'", show(&typ));
        } else {
            report_chunk(rep, &typ, data);
        }
        p += total;
        if &typ == b"IEND" {
            seen_end = true;
            break;
        }
    }
    if !seen_end {
        bail!("PNG is truncated (no IEND chunk)");
    }
    if p < d.len() {
        rep.add("Hidden data after image", vec![("Size".into(), format!("{} bytes", d.len() - p))]);
    }
    Ok(out)
}

fn report_chunk(rep: &mut Report, typ: &[u8; 4], data: &[u8]) {
    let keyword = |d: &[u8]| show(&d[..d.iter().position(|&b| b == 0).unwrap_or(d.len())]);
    match typ {
        b"tEXt" => {
            let k = keyword(data);
            let v = data.iter().position(|&b| b == 0).map(|i| show(&data[i + 1..])).unwrap_or_default();
            push(rep, "Text fields", (k, v));
        }
        b"iTXt" => {
            let k = keyword(data);
            // keyword\0 flag method lang\0 translated\0 text
            let text = (|| {
                let k_end = data.iter().position(|&b| b == 0)?;
                if *data.get(k_end + 1)? != 0 {
                    return Some("(compressed)".to_string());
                }
                let rest = data.get(k_end + 3..)?;
                let l = rest.iter().position(|&b| b == 0)?;
                let rest = &rest[l + 1..];
                let t = rest.iter().position(|&b| b == 0)?;
                Some(show(&rest[t + 1..]))
            })()
            .unwrap_or_default();
            if k == "XML:com.adobe.xmp" {
                push(rep, "XMP", ("".into(), "".into()));
            } else {
                push(rep, "Text fields", (k, text));
            }
        }
        b"zTXt" => push(rep, "Text fields", (keyword(data), "(compressed)".into())),
        b"eXIf" => {
            let fields = tiff::parse(data).map(|e| e.fields).unwrap_or_default();
            rep.add("EXIF", fields);
        }
        b"tIME" if data.len() >= 7 => {
            let y = u16::from_be_bytes([data[0], data[1]]);
            let t = format!("{y}-{:02}-{:02} {:02}:{:02}", data[2], data[3], data[4], data[5]);
            rep.add("Timestamp", vec![("Modified".into(), t)]);
        }
        _ => push(rep, "Other chunks", ("Chunk".into(), show(typ))),
    }
}

fn push(rep: &mut Report, label: &str, field: (String, String)) {
    let item = match rep.removed.iter().position(|i| i.label == label) {
        Some(i) => &mut rep.removed[i],
        None => {
            rep.add(label, vec![]);
            rep.removed.last_mut().unwrap()
        }
    };
    if !field.0.is_empty() {
        item.fields.push(field);
    }
}
