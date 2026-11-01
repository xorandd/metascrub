//! metascrub-cli: remove hidden metadata (GPS, camera, author names...) from files.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use metascrub_core::{self as core, Options, Report};

#[derive(Parser)]
#[command(
    name = "metascrub-cli",
    version,
    about = "Remove hidden metadata (GPS location, camera, author names, edit history) from photos and documents.",
    after_help = "Cleaned files are saved next to the original as cleaned_<name>; the original is never changed.\n\
                  Supported: JPEG, PNG, WebP, GIF, PDF, DOCX, XLSX, PPTX.\n\n\
                  Examples:\n  metascrub-cli photo.jpg\n  metascrub-cli photo.jpg -o holiday\n  metascrub-cli --show report.pdf\n  metascrub-cli -r ~/Pictures/to-share"
)]
struct Cli {
    /// Files (or folders, with -r) to clean
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// Name for the cleaned file (only with a single input file)
    #[arg(short, long, value_name = "NAME")]
    output: Option<String>,

    /// Only show the metadata that was found; don't write anything
    #[arg(short, long)]
    show: bool,

    /// Overwrite the output file if it already exists (only matters with -o)
    #[arg(short, long)]
    force: bool,

    /// Process folders and everything inside them
    #[arg(short, long)]
    recursive: bool,

    /// Also remove the photo rotation flag (photos may then appear sideways)
    #[arg(long)]
    strip_orientation: bool,

    /// Print only errors
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let opts = Options { keep_orientation: !cli.strip_orientation };

    let mut files = Vec::new();
    for p in &cli.files {
        collect(p, cli.recursive, &mut files);
    }
    if cli.output.is_some() && files.len() != 1 {
        eprintln!("error: -o/--output can only be used with exactly one file");
        return ExitCode::from(2);
    }
    if files.is_empty() {
        eprintln!("error: no files to process");
        return ExitCode::from(2);
    }

    let mut failed = 0;
    for f in &files {
        let result = if cli.show {
            core::inspect_file(f, &opts).map(|r| (None, r))
        } else {
            core::clean_file(f, cli.output.as_deref(), cli.force, &opts).map(|(o, r)| (Some(o), r))
        };
        match result {
            Ok((out, rep)) if !cli.quiet => print_report(f, out.as_deref(), &rep, cli.show),
            Ok(_) => {}
            Err(e) => {
                failed += 1;
                eprintln!("✗ {}: {e:#}", f.display());
            }
        }
    }

    if !cli.quiet && files.len() > 1 {
        println!("\n{} processed, {} failed", files.len() - failed, failed);
    }
    if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Expand folders into the supported files inside them.
fn collect(p: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        if recursive {
            out.extend(core::files_in_folder(p));
        } else {
            eprintln!("skipping folder {} (use -r to include folders)", p.display());
        }
    } else {
        out.push(p.to_path_buf()); // explicit files are always tried
    }
}

fn print_report(file: &Path, out: Option<&Path>, rep: &Report, show_only: bool) {
    let name = file.display();
    match (out, show_only) {
        (_, true) if rep.is_clean() => println!("✓ {name} ({}): no metadata found", rep.format.name()),
        (_, true) => println!("● {name} ({}): metadata found", rep.format.name()),
        (Some(o), _) if rep.is_clean() => println!("✓ {name} → {} (nothing to remove)", o.display()),
        (Some(o), _) => println!("✓ {name} → {}", o.display()),
        (None, false) => {}
    }
    for item in &rep.removed {
        let verb = if show_only { "" } else { "removed " };
        println!("    {verb}{}", item.label);
        for (k, v) in &item.fields {
            if v.is_empty() {
                println!("      {k}");
            } else {
                println!("      {k}: {v}");
            }
        }
    }
    for n in &rep.notes {
        println!("    note: {n}");
    }
}
