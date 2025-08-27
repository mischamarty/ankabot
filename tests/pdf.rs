use std::fs;
use std::process::Command;

#[test]
fn saves_pdf_when_requested() {
    let out_path = std::env::temp_dir().join("ankabot_test.pdf");
    let _ = fs::remove_file(&out_path);
    let status = Command::new(env!("CARGO_BIN_EXE_ankabot"))
        .args(["--pdf", out_path.to_str().unwrap(), "https://example.com"])
        .status()
        .expect("run ankabot");
    assert!(status.success(), "ankabot failed");
    let meta = fs::metadata(&out_path).expect("pdf missing");
    assert!(meta.len() > 10 * 1024, "pdf too small");
    let _ = fs::remove_file(&out_path);
}

#[test]
fn no_pdf_when_disabled() {
    let output = Command::new(env!("CARGO_BIN_EXE_ankabot"))
        .args(["--no-pdf", "https://example.com"])
        .output()
        .expect("run ankabot");
    assert!(output.status.success(), "ankabot failed");
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert!(v.get("pdf_path").and_then(|p| p.as_null()).is_some());
}
