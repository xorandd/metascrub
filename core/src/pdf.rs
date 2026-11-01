//! PDF: remove the document info dictionary, XMP metadata streams, editing
//! app data (PieceInfo) and the file ID, then write a brand-new file.
//!
//! Writing a fresh file matters: tools that use an "incremental update" leave
//! the old metadata in the file where anyone can still recover it.

use anyhow::{Result, anyhow, bail};
use lopdf::{Dictionary, Document, Object};

use crate::{Report, show};

pub fn clean(d: &[u8], rep: &mut Report) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(d).map_err(|e| anyhow!("cannot read PDF: {e}"))?;
    if doc.is_encrypted() || doc.trailer.has(b"Encrypt") {
        bail!("password-protected / encrypted PDFs are not supported yet");
    }

    if let Some(info) = doc.trailer.remove(b"Info") {
        let dict = match &info {
            Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()).cloned(),
            Object::Dictionary(d) => Some(d.clone()),
            _ => None,
        };
        let fields = dict.as_ref().map(info_fields).unwrap_or_default();
        rep.add("Document info", fields);
    }
    if doc.trailer.remove(b"ID").is_some() {
        rep.add("File ID", vec![]);
    }

    let (mut xmp, mut piece) = (0, 0);
    let mut annot_authors: Vec<String> = Vec::new();
    for obj in doc.objects.values_mut() {
        let dict = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        if dict.remove(b"Metadata").is_some() {
            xmp += 1;
        }
        if dict.remove(b"PieceInfo").is_some() {
            piece += 1;
            dict.remove(b"LastModified");
        }
        // Comment/markup annotations store the author's name in /T and edit
        // times in /M. Form-field widgets also use /T (as the field name), so
        // those are left alone.
        let is_markup = dict.has(b"Rect")
            && matches!(dict.get(b"Subtype"), Ok(Object::Name(n)) if n.as_slice() != b"Widget" && n.as_slice() != b"Link");
        if is_markup {
            if let Some(Object::String(b, _)) = dict.remove(b"T") {
                let name = decode(&b);
                if !name.is_empty() && !annot_authors.contains(&name) {
                    annot_authors.push(name);
                }
            }
            dict.remove(b"M");
            dict.remove(b"CreationDate");
        }
    }
    if !annot_authors.is_empty() {
        rep.add("Comment authors", annot_authors.into_iter().map(|a| ("Name".to_string(), a)).collect());
    }
    if xmp > 0 {
        rep.add("XMP metadata", vec![("Blocks".into(), xmp.to_string())]);
    }
    if piece > 0 {
        rep.add("Editing app data", vec![("Blocks".into(), piece.to_string())]);
    }

    // Drops the now-unreferenced info dictionary and XMP streams.
    doc.prune_objects();
    let mut out = Vec::with_capacity(d.len());
    doc.save_to(&mut out).map_err(|e| anyhow!("cannot write PDF: {e}"))?;
    Ok(out)
}

fn info_fields(dict: &Dictionary) -> Vec<(String, String)> {
    dict.iter()
        .filter_map(|(k, v)| {
            let Object::String(bytes, _) = v else { return None };
            let value = decode(bytes);
            (!value.is_empty()).then(|| (String::from_utf8_lossy(k).into_owned(), value))
        })
        .collect()
}

/// PDF text strings are UTF-16BE (with a BOM) or a Latin-1-like encoding.
fn decode(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        show(String::from_utf16_lossy(&units).as_bytes())
    } else if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        show(&b[3..])
    } else {
        show(b.iter().map(|&c| c as char).collect::<String>().as_bytes())
    }
}
