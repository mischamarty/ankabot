use std::process::Command;

#[test]
fn reports_on_timeout() {
    let output = Command::new(env!("CARGO_BIN_EXE_ankabot"))
        .args([
            "--max-wait-ms",
            "1",
            "--on-timeout",
            "report",
            "https://example.com",
        ])
        .output()
        .expect("run ankabot");
    assert_eq!(output.status.code(), Some(2));
    let path = std::str::from_utf8(&output.stdout).unwrap().trim();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("json");
    assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("timeout"));
    assert!(v
        .get("artifacts")
        .and_then(|a| a.get("pdf"))
        .and_then(|p| p.as_str())
        .is_some());
}
