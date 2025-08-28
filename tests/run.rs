use std::fs;
use std::process::Command;

#[test]
fn saves_pdf_and_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_ankabot"))
        .arg("https://example.com")
        .output()
        .expect("run ankabot");
    assert!(output.status.success(), "ankabot failed");
    let path = std::str::from_utf8(&output.stdout).unwrap().trim();
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).expect("json");
    let pdf = v
        .get("pdf_path")
        .and_then(|p| p.as_str())
        .expect("pdf path");
    let meta = fs::metadata(pdf).expect("pdf missing");
    assert!(meta.len() > 10 * 1024, "pdf too small");
}

#[test]
fn run_dir_override_creates_folder() {
    let run_dir = std::env::temp_dir().join("ankabot_custom_run");
    let _ = fs::remove_dir_all(&run_dir);
    let output = Command::new(env!("CARGO_BIN_EXE_ankabot"))
        .args([
            "--run-dir",
            run_dir.to_str().unwrap(),
            "https://example.com",
        ])
        .output()
        .expect("run ankabot");
    assert!(output.status.success(), "ankabot failed");
    let path = std::str::from_utf8(&output.stdout).unwrap().trim();
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).expect("json");
    let canon = dunce::canonicalize(&run_dir).unwrap();
    assert_eq!(
        v.get("run_dir").and_then(|p| p.as_str()),
        Some(canon.to_str().unwrap())
    );
}
