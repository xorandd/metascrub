use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use metascrub_core::{Options, clean_bytes, detect, output_path};

const FIXTURES: &[&str] =
    &["photo.jpg", "pic.png", "pic.webp", "anim.gif", "doc.pdf", "doc.docx", "sheet.xlsx", "deck.pptx"];
const SECRETS: &[&[u8]] = &[b"Yaroslav", b"secret note", b"HIDDEN-TRAILER", b"EOS R5", b"made by me", b"Secret plan"];

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

#[test]
fn every_fixture_has_metadata_and_cleans_completely() {
    for name in FIXTURES {
        let data = fixture(name);
        let (out, rep) = clean_bytes(&data, &Options::default()).unwrap();
        assert!(!rep.is_clean(), "{name}: expected metadata to be found");
        assert_eq!(detect(&out), detect(&data), "{name}: format changed");
        // Cleaning twice finds nothing new.
        let (_, rep2) = clean_bytes(&out, &Options::default()).unwrap();
        assert!(rep2.is_clean(), "{name}: second pass still found {:?}", rep2.removed);
        // Plain-text secrets are gone (office files are compressed, checked elsewhere).
        if !name.ends_with('x') {
            for s in SECRETS {
                assert!(
                    !out.windows(s.len()).any(|w| w == *s),
                    "{name}: still contains {}",
                    String::from_utf8_lossy(s)
                );
            }
        }
    }
}

#[test]
fn jpeg_keeps_only_rotation_unless_asked() {
    let data = fixture("photo.jpg");
    let (kept, _) = clean_bytes(&data, &Options { keep_orientation: true }).unwrap();
    let (stripped, _) = clean_bytes(&data, &Options { keep_orientation: false }).unwrap();
    assert!(kept.windows(6).any(|w| w == b"Exif\0\0"));
    assert!(!stripped.windows(6).any(|w| w == b"Exif\0\0"));
}

static PANICS: AtomicUsize = AtomicUsize::new(0);

/// Truncated and corrupted files must give an error or a result, never a crash.
#[test]
fn damaged_files_never_crash() {
    std::panic::set_hook(Box::new(|info| {
        PANICS.fetch_add(1, Ordering::SeqCst);
        eprintln!("panic: {info}");
    }));
    let mut seed: u64 = 0x2545F4914F6CDD1D;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for name in FIXTURES {
        let data = fixture(name);
        let step = (data.len() / 150).max(1);
        for cut in (0..data.len()).step_by(step) {
            let _ = clean_bytes(&data[..cut], &Options::default());
        }
        for _ in 0..300 {
            let mut d = data.clone();
            for _ in 0..1 + rand() % 8 {
                let i = (rand() as usize) % d.len();
                d[i] = rand() as u8;
            }
            let _ = clean_bytes(&d, &Options::default());
        }
    }
    let _ = std::panic::take_hook();
    assert_eq!(PANICS.load(Ordering::SeqCst), 0, "some inputs caused panics (see log)");
}

#[test]
fn output_names() {
    let dir = std::env::temp_dir().join(format!("metascrub-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("photo.JPG");
    assert_eq!(output_path(&input, None), dir.join("cleaned_photo.JPG"));
    std::fs::write(dir.join("cleaned_photo.JPG"), b"x").unwrap();
    assert_eq!(output_path(&input, None), dir.join("cleaned_photo_2.JPG"));
    assert_eq!(output_path(&input, Some("holiday")), dir.join("holiday.JPG"));
    assert_eq!(output_path(&input, Some("holiday.jpeg")), dir.join("holiday.jpeg"));
    assert_eq!(output_path(&input, Some("out/x.jpg")), PathBuf::from("out/x.jpg"));
    std::fs::remove_dir_all(&dir).unwrap();
}
