//! Office Open XML (docx / xlsx / pptx): blank the document properties, replace
//! comment and tracked-change author names, remove revision IDs and the saved
//! folder path, and reset every timestamp inside the zip.

use std::io::{Cursor, Read, Write};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::{Captures, Regex};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::{Report, show};

pub fn is_office(d: &[u8]) -> bool {
    ZipArchive::new(Cursor::new(d)).is_ok_and(|mut z| z.by_name("[Content_Types].xml").is_ok())
}

const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"></cp:coreProperties>"#;
const APP: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"></Properties>"#;
const CUSTOM: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"></Properties>"#;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(CORE_FIELD, r"<(?:dc|cp|dcterms):(\w+)[^>]*>([^<]+)</");
re!(APP_FIELD, r"<(Company|Manager|Template|TotalTime|HyperlinkBase)>([^<]+)</");
re!(CUSTOM_NAME, r#"<property [^>]*name="([^"]*)""#);
// Author-like attributes in Word, PowerPoint and Excel threaded comments.
re!(AUTHOR_ATTR, r#"\b(w:author|w15:author|w:initials|displayName|initials)="([^"]*)""#);
// Account IDs: values are blanked, attributes kept (some schemas require them).
re!(USER_ID_ATTR, r#"\b(w15:userId|userId)="[^"]*""#);
re!(PPT_NAME_ATTR, r#"\bname="([^"]*)""#);
re!(XL_AUTHOR, r"<author>([^<]*)</author>");
re!(RSID_ATTR, r#"\sw:rsid\w*="[0-9A-Fa-f]*""#);
re!(RSIDS_BLOCK, r"(?s)<w:rsids>.*?</w:rsids>");
re!(DATE_ATTR, r#"\sw:date="[^"]*""#);
re!(ABS_PATH, r#"<[\w]+:absPath\s[^>]*/>"#);

pub fn clean(d: &[u8], rep: &mut Report) -> Result<Vec<u8>> {
    let mut zin = ZipArchive::new(Cursor::new(d)).context("cannot open Office file")?;
    let mut zout = ZipWriter::new(Cursor::new(Vec::with_capacity(d.len())));
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default());

    let mut authors: Vec<String> = Vec::new();
    let (mut rsids, mut paths) = (false, false);

    for i in 0..zin.len() {
        let mut f = zin.by_index(i)?;
        let name = f.name()?.to_string();
        if f.is_dir() {
            zout.add_directory(name, opts)?;
            continue;
        }
        // Don't trust the declared size for the allocation (zip bombs).
        let mut buf = Vec::with_capacity((f.size() as usize).min(16 << 20));
        f.read_to_end(&mut buf).with_context(|| format!("cannot read {name} inside the file"))?;
        drop(f);

        let new: Vec<u8> = match name.as_str() {
            "docProps/core.xml" => {
                let fields = captures(&CORE_FIELD, &buf);
                if !fields.is_empty() {
                    rep.add("Document properties", fields);
                }
                CORE.into()
            }
            "docProps/app.xml" => {
                let fields = captures(&APP_FIELD, &buf);
                if !fields.is_empty() {
                    rep.add("App properties", fields);
                }
                APP.into()
            }
            "docProps/custom.xml" => {
                let s = String::from_utf8_lossy(&buf);
                let names: Vec<_> =
                    CUSTOM_NAME.captures_iter(&s).map(|c| ("Property".into(), show(c[1].as_bytes()))).collect();
                if !names.is_empty() {
                    rep.add("Custom properties", names);
                }
                CUSTOM.into()
            }
            n if is_part(n) => match String::from_utf8(buf) {
                Ok(s) => scrub_part(n, s, &mut authors, &mut rsids, &mut paths).into_bytes(),
                Err(e) => e.into_bytes(), // not UTF-8: leave untouched
            },
            _ => buf,
        };
        zout.start_file(name, opts)?;
        zout.write_all(&new)?;
    }

    if !authors.is_empty() {
        let fields = authors.into_iter().map(|a| ("Name".to_string(), a)).collect();
        rep.add("Comment / revision authors", fields);
    }
    if rsids {
        rep.add("Revision IDs", vec![]);
    }
    if paths {
        rep.add("Saved folder path", vec![]);
    }
    Ok(zout.finish()?.into_inner())
}

fn is_part(n: &str) -> bool {
    (n.starts_with("word/") || n.starts_with("xl/") || n.starts_with("ppt/")) && n.ends_with(".xml")
}

fn captures(re: &Regex, buf: &[u8]) -> Vec<(String, String)> {
    let s = String::from_utf8_lossy(buf);
    re.captures_iter(&s).map(|c| (c[1].to_string(), show(c[2].as_bytes()))).filter(|(_, v)| !v.is_empty()).collect()
}

fn note(authors: &mut Vec<String>, v: &str) {
    let v = show(v.as_bytes());
    if !v.is_empty() && v != "Author" && v != "A" && !authors.contains(&v) {
        authors.push(v);
    }
}

fn scrub_part(name: &str, s: String, authors: &mut Vec<String>, rsids: &mut bool, paths: &mut bool) -> String {
    let mut s = s;
    {
        s = AUTHOR_ATTR
            .replace_all(&s, |c: &Captures| {
                let attr = &c[1];
                // `initials` / `displayName` are only author data in comment files.
                let comment_file = name.contains("comment")
                    || name.contains("people")
                    || name.contains("person")
                    || name.contains("authors");
                if (attr == "displayName" || attr == "initials") && !comment_file {
                    return c[0].to_string();
                }
                if !attr.ends_with("initials") {
                    note(authors, &c[2]);
                }
                let generic = if attr.ends_with("initials") { "A" } else { "Author" };
                format!(r#"{attr}="{generic}""#)
            })
            .into_owned();
        if name.contains("people") || name.contains("person") || name.contains("authors") {
            s = USER_ID_ATTR.replace_all(&s, r#"$1="""#).into_owned();
        }
    }
    if name.starts_with("ppt/") && (name.ends_with("commentAuthors.xml") || name.ends_with("authors.xml")) {
        s = PPT_NAME_ATTR
            .replace_all(&s, |c: &Captures| {
                note(authors, &c[1]);
                r#"name="Author""#.to_string()
            })
            .into_owned();
    }
    if name.starts_with("xl/") && name.contains("comments") {
        s = XL_AUTHOR
            .replace_all(&s, |c: &Captures| {
                note(authors, &c[1]);
                "<author>Author</author>".to_string()
            })
            .into_owned();
    }
    if name.starts_with("word/") {
        if RSID_ATTR.is_match(&s) || RSIDS_BLOCK.is_match(&s) {
            *rsids = true;
            s = RSID_ATTR.replace_all(&s, "").into_owned();
            s = RSIDS_BLOCK.replace_all(&s, "").into_owned();
        }
        s = DATE_ATTR.replace_all(&s, "").into_owned();
    }
    if ABS_PATH.is_match(&s) {
        *paths = true;
        s = ABS_PATH.replace_all(&s, "").into_owned();
    }
    s
}
