//! End-to-end smoke test for the DOCX path. Exercises the binary against
//! the committed fixture `tests/fixtures/mixed.docx` and verifies:
//!   - Bijoy SutonnyMJ run is converted to Unicode Bengali.
//!   - Times New Roman English run is byte-identical in the output.
//!   - Existing Unicode Bengali (Kalpurush) run is preserved.
//!   - The audit log has one JSONL entry per processed run.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points to the crate dir; ../.. is the workspace.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn binary_path() -> PathBuf {
    // The integration test binary lives under
    // target/<profile>/deps/<test>-<hash>; the CLI binary is in target/<profile>/.
    // `env!("CARGO_BIN_EXE_<name>")` is the canonical way.
    PathBuf::from(env!("CARGO_BIN_EXE_banglakit-converter"))
}

#[test]
fn docx_end_to_end() {
    let fixture = workspace_root().join("tests/fixtures/mixed.docx");
    assert!(fixture.exists(), "missing fixture {}", fixture.display());

    let out = tempfile("docx_e2e_out", ".docx");
    let audit = tempfile("docx_e2e_audit", ".jsonl");

    let status = Command::new(binary_path())
        .arg("-i")
        .arg(&fixture)
        .arg("-o")
        .arg(&out)
        .arg("--audit")
        .arg(&audit)
        .status()
        .expect("spawn CLI");
    // Exit code 1 = changes made (PRD FR-9).
    assert!(matches!(status.code(), Some(0) | Some(1)), "status: {status}");

    // Inspect output document.xml.
    let bytes = std::fs::read(&out).expect("read output");
    let xml = unzip_document_xml(&bytes);
    assert!(xml.contains("আমি বাংলায়"), "Bijoy run not converted: {xml}");
    assert!(xml.contains("Kalpurush"), "target font not written");
    assert!(
        xml.contains("Source: Daily Star, 2023"),
        "English run mutated: {xml}"
    );
    assert!(
        xml.contains("Times New Roman"),
        "English font dropped: {xml}"
    );

    // Audit log: one line per processed run; in this fixture, four runs.
    let audit_text = std::fs::read_to_string(&audit).expect("read audit");
    let lines: Vec<&str> = audit_text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 4, "audit lines: {lines:?}");
    assert!(lines[0].contains("\"decision\":\"ansi_bengali\""));
    assert!(lines[1].contains("\"decision\":\"latin\""));
    assert!(lines[3].contains("\"decision\":\"unicode_bengali\""));

    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&audit);
}

#[test]
fn plain_text_canonical_sample() {
    // PRD §2 canonical example via stdin in safe mode.
    let input = "Avwg evsjvq Mvb MvB|";
    let out = std::process::Command::new(binary_path())
        .arg("-i")
        .arg("-")
        .arg("-o")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .expect("run CLI");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.starts_with("আমি বাংলায়"), "stdout: {stdout:?}");
}

#[test]
fn english_is_untouched_in_safe_mode() {
    let input = "Gas price is $5 today";
    let out = std::process::Command::new(binary_path())
        .arg("-i")
        .arg("-")
        .arg("-o")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .expect("run CLI");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout.trim_end(), input);
    assert_eq!(out.status.code(), Some(0), "expected exit 0 (no change)");
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

fn unzip_document_xml(bytes: &[u8]) -> String {
    use std::io::Read;
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).expect("zip parse");
    let mut entry = archive.by_name("word/document.xml").expect("document.xml");
    let mut s = String::new();
    entry.read_to_string(&mut s).expect("read xml");
    s
}
