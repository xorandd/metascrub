//! GIF: drop comment blocks and application blocks other than looping (NETSCAPE)
//! and colour profiles (ICC); drop anything after the trailer.

use anyhow::{Result, bail};

use crate::{Report, show};

pub fn clean(d: &[u8], rep: &mut Report) -> Result<Vec<u8>> {
    let truncated = || anyhow::anyhow!("GIF is truncated");
    if d.len() < 13 {
        return Err(truncated());
    }
    let mut p = 13;
    if d[10] & 0x80 != 0 {
        p += 3 << ((d[10] & 0x07) + 1); // global colour table
    }
    if p > d.len() {
        return Err(truncated());
    }
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&d[..p]);

    loop {
        let Some(&b) = d.get(p) else { return Err(truncated()) };
        match b {
            0x3B => {
                out.push(0x3B);
                p += 1;
                break;
            }
            0x2C => {
                // Image descriptor (10 bytes), optional local colour table,
                // LZW code size byte, then data sub-blocks.
                let start = p;
                let flags = *d.get(p + 9).ok_or_else(truncated)?;
                p += 10;
                if flags & 0x80 != 0 {
                    p += 3 << ((flags & 0x07) + 1);
                }
                p += 1;
                p = skip_blocks(d, p).ok_or_else(truncated)?;
                out.extend_from_slice(&d[start..p]);
            }
            0x21 => {
                let start = p;
                let label = *d.get(p + 1).ok_or_else(truncated)?;
                let first = d.get(p + 2..).ok_or_else(truncated)?;
                let end = skip_blocks(d, p + 2).ok_or_else(truncated)?;
                match label {
                    0xFE => rep.add("Comment", vec![("Text".into(), show(&blocks_text(first)))]),
                    0xFF => {
                        let id = first.get(1..12).unwrap_or_default();
                        if id.starts_with(b"NETSCAPE2.0")
                            || id.starts_with(b"ANIMEXTS1.0")
                            || id.starts_with(b"ICCRGBG1")
                        {
                            out.extend_from_slice(&d[start..end]);
                        } else if id.starts_with(b"XMP DataXMP") {
                            rep.add("XMP", vec![]);
                        } else {
                            rep.add("Application block", vec![("Id".into(), show(id))]);
                        }
                    }
                    _ => out.extend_from_slice(&d[start..end]), // graphic control, plain text
                }
                p = end;
            }
            _ => bail!("GIF is corrupt (unexpected byte {b:#04x})"),
        }
    }
    if p < d.len() {
        rep.add("Hidden data after image", vec![("Size".into(), format!("{} bytes", d.len() - p))]);
    }
    Ok(out)
}

/// Skip a run of sub-blocks starting at `p`; returns the index after the terminator.
fn skip_blocks(d: &[u8], mut p: usize) -> Option<usize> {
    loop {
        let n = *d.get(p)? as usize;
        p += 1;
        if n == 0 {
            return Some(p);
        }
        p += n;
        if p > d.len() {
            return None;
        }
    }
}

fn blocks_text(d: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    let mut p = 0;
    while let Some(&n) = d.get(p) {
        if n == 0 {
            break;
        }
        v.extend_from_slice(d.get(p + 1..p + 1 + n as usize).unwrap_or_default());
        p += 1 + n as usize;
    }
    v
}
