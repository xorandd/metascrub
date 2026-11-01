//! metascrub-core: strip metadata from files without external tools.
//!
//! Every cleaner works on bytes in memory and rebuilds the file from scratch,
//! so removed metadata is really gone (no "incremental update" leftovers).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

mod gif;
mod jpeg;
mod office;
mod pdf;
mod png;
mod tiff;
mod webp;

/// Prefix added to cleaned files when no custom name is given.
pub const PREFIX: &str = "cleaned_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    WebP,
    Gif,
    Pdf,
    Office,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::WebP => "WebP",
            Format::Gif => "GIF",
            Format::Pdf => "PDF",
            Format::Office => "Office (docx/xlsx/pptx)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Keep the JPEG orientation flag so photos don't appear rotated.
    pub keep_orientation: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { keep_orientation: true }
    }
}

/// One piece of metadata that was found and removed.
#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    /// Human-readable fields found inside it (e.g. "GPS" -> "50.45, 30.52").
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub format: Format,
    pub removed: Vec<Item>,
    pub notes: Vec<String>,
}

impl Report {
    fn new(format: Format) -> Self {
        Report { format, removed: Vec::new(), notes: Vec::new() }
    }
    pub(crate) fn add(&mut self, label: impl Into<String>, fields: Vec<(String, String)>) {
        self.removed.push(Item { label: label.into(), fields });
    }
    /// True when the file had no metadata worth removing.
    pub fn is_clean(&self) -> bool {
        self.removed.is_empty()
    }
}

/// Detect the file format from its first bytes (extensions are ignored).
pub fn detect(data: &[u8]) -> Option<Format> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(Format::WebP)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if data[..data.len().min(1024)].windows(5).any(|w| w == b"%PDF-") {
        Some(Format::Pdf)
    } else if data.starts_with(b"PK\x03\x04") && office::is_office(data) {
        Some(Format::Office)
    } else {
        None
    }
}

/// Clean a file held in memory. Returns the cleaned bytes and what was removed.
pub fn clean_bytes(data: &[u8], opts: &Options) -> Result<(Vec<u8>, Report)> {
    let Some(format) = detect(data) else {
        bail!("unsupported file type (supported: JPEG, PNG, WebP, GIF, PDF, DOCX, XLSX, PPTX)");
    };
    // Our parsers are bounds-checked, but third-party libraries (PDF, zip) can
    // still panic on hostile input. Turn that into a normal error so one bad
    // file never takes down the app or a batch run.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut rep = Report::new(format);
        let out = match format {
            Format::Jpeg => jpeg::clean(data, opts, &mut rep)?,
            Format::Png => png::clean(data, &mut rep)?,
            Format::WebP => webp::clean(data, &mut rep)?,
            Format::Gif => gif::clean(data, &mut rep)?,
            Format::Pdf => pdf::clean(data, &mut rep)?,
            Format::Office => office::clean(data, &mut rep)?,
        };
        Ok((out, rep))
    }));
    result.unwrap_or_else(|_| bail!("the {} file is malformed and could not be processed", format.name()))
}

/// Inspect a file: report what *would* be removed, without writing anything.
pub fn inspect_file(input: &Path, opts: &Options) -> Result<Report> {
    let data = fs::read(input).with_context(|| format!("cannot read {}", input.display()))?;
    Ok(clean_bytes(&data, opts)?.1)
}

/// Work out where the cleaned file goes.
///
/// * No name: `cleaned_<original name>` next to the original. If that already
///   exists, `cleaned_<stem>_2.<ext>`, `_3`, ... is used so nothing is overwritten.
/// * Custom name without a folder: placed next to the original. If it has no
///   extension, the original extension is added.
/// * Custom name with a folder: used as given.
pub fn output_path(input: &Path, custom: Option<&str>) -> PathBuf {
    let dir = input.parent().map(Path::to_path_buf).unwrap_or_default();
    let ext = input.extension().map(|e| e.to_string_lossy().into_owned());

    if let Some(name) = custom {
        let mut p = PathBuf::from(name);
        if p.extension().is_none()
            && let Some(e) = &ext
        {
            p.set_extension(e);
        }
        return if p.components().count() > 1 || p.is_absolute() { p } else { dir.join(p) };
    }

    let file_name = input.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let first = dir.join(format!("{PREFIX}{file_name}"));
    if !first.exists() {
        return first;
    }
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    (2..)
        .map(|n| {
            let name = match &ext {
                Some(e) => format!("{PREFIX}{stem}_{n}.{e}"),
                None => format!("{PREFIX}{stem}_{n}"),
            };
            dir.join(name)
        })
        .find(|p| !p.exists())
        .expect("infinite iterator")
}

/// Clean `input` and write the result. Returns the output path and the report.
///
/// `overwrite` only matters for custom names; auto-generated names never clash.
pub fn clean_file(
    input: &Path,
    custom_name: Option<&str>,
    overwrite: bool,
    opts: &Options,
) -> Result<(PathBuf, Report)> {
    let data = fs::read(input).with_context(|| format!("cannot read {}", input.display()))?;
    let (cleaned, rep) = clean_bytes(&data, opts)?;
    let out = output_path(input, custom_name);
    if out.exists() && !overwrite {
        bail!("{} already exists (use --force to overwrite)", out.display());
    }
    if same_file(input, &out) {
        bail!("output would overwrite the original file");
    }
    fs::write(&out, cleaned).with_context(|| format!("cannot write {}", out.display()))?;
    Ok((out, rep))
}

/// File extensions picked up when walking a folder. (Content is still checked
/// when cleaning; this only avoids reading every unrelated file.)
pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "jfif", "png", "webp", "gif", "pdf", "docx", "xlsx", "pptx"];

/// All supported files inside `dir` and its subfolders, sorted. Hidden files
/// and files we produced ourselves (`cleaned_*`) are skipped.
pub fn files_in_folder(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            // file_type() does not follow symlinks, so link loops can't trap us.
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() && !name.starts_with(PREFIX) && is_supported_name(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn is_supported_name(p: &Path) -> bool {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Lossy text decode for report output, shortened for display.
pub(crate) fn show(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    let mut s: String = s.chars().filter(|c| !c.is_control()).collect();
    if s.chars().count() > 80 {
        s = s.chars().take(77).collect::<String>() + "...";
    }
    s
}
