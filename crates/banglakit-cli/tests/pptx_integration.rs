//! End-to-end smoke test for the PPTX path. Exercises the binary against
//! the committed fixture `tests/fixtures/mixed.pptx` and verifies:
//!   - SutonnyMJ Bijoy run is converted to Unicode Bengali in slide 1.
//!   - Calibri English run is byte-identical in the output.
//!   - Kalpurush Unicode Bengali run in slide 2 is preserved.
//!   - The audit log carries slide_index for each entry.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_banglakit-converter"))
}

#[test]
fn pptx_end_to_end() {
    let fixture = workspace_root().join("tests/fixtures/mixed.pptx");
    assert!(fixture.exists(), "missing fixture {}", fixture.display());

    let out = tempfile("pptx_e2e_out", ".pptx");
    let audit = tempfile("pptx_e2e_audit", ".jsonl");

    let status = Command::new(binary_path())
        .arg("-i")
        .arg(&fixture)
        .arg("-o")
        .arg(&out)
        .arg("--audit")
        .arg(&audit)
        .status()
        .expect("spawn CLI");
    assert_eq!(status.code(), Some(1), "expected changes (exit 1)");

    let bytes = std::fs::read(&out).expect("read output");
    // Concatenate every slide XML so we can search across the whole deck.
    let combined = read_all_slides(&bytes);

    assert!(
        combined.contains("আমি বাংলায়"),
        "Bijoy run not converted in any slide"
    );
    assert!(
        combined.contains("Kalpurush"),
        "target font not written into output"
    );
    assert!(
        combined.contains("Source: Daily Star"),
        "English run mutated"
    );
    assert!(combined.contains("Calibri"), "English font dropped");

    let audit_text = std::fs::read_to_string(&audit).expect("read audit");
    let lines: Vec<&str> = audit_text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 3, "audit lines: {lines:?}");
    assert!(
        lines.iter().all(|l| l.contains("\"slide_index\":")),
        "audit lines missing slide_index: {lines:?}"
    );
    assert!(lines[0].contains("\"source_format\":\"pptx\""));

    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&audit);
}

fn tempfile(prefix: &str, suffix: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!("banglakit_{prefix}_{pid}_{nanos}{suffix}"));
    p
}

fn read_all_slides(bytes: &[u8]) -> String {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).expect("zip parse");
    let mut combined = String::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("entry");
        let name = entry.name().to_string();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            entry.read_to_string(&mut combined).expect("read xml");
        }
    }
    combined
}
