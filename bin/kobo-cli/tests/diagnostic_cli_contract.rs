use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_fixture_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{label}-{}-{nonce}", std::process::id()))
}

fn write_kobo_fixture(label: &str, file_name: &str, source: &str) -> (PathBuf, PathBuf) {
    let root = unique_fixture_dir(label);
    fs::create_dir_all(&root).unwrap();
    let file = root.join(file_name);
    fs::write(&file, source).unwrap();
    (root, file)
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn explain_prints_registry_text_for_k0107() {
    let exe = env!("CARGO_BIN_EXE_kobo");
    let output = Command::new(exe)
        .args(["explain", "K0107"])
        .output()
        .expect("kobo explain should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("K0107"));
    assert!(stdout.contains("external crate"));
    assert!(stdout.contains("replay"));
}

#[test]
fn check_bad_parse_uses_registry_backed_card() {
    let exe = env!("CARGO_BIN_EXE_kobo");
    let (root, file) =
        write_kobo_fixture("kobo-cli-parse-card", "bad.kobo", "fn main() { let x = }\n");

    let output = Command::new(exe)
        .args(["check", file.to_str().unwrap()])
        .output()
        .expect("kobo check should run");
    let text = combined_output(&output);

    assert!(!output.status.success());
    assert!(
        text.contains("error[K011"),
        "expected parser K011x card, got:\n{text}"
    );
    assert!(
        text.contains("-->"),
        "expected source label in diagnostic card, got:\n{text}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_bad_parse_json_uses_typed_schema() {
    let exe = env!("CARGO_BIN_EXE_kobo");
    let (root, file) = write_kobo_fixture(
        "kobo-cli-parse-json",
        "bad_json.kobo",
        "fn main() { let x = }\n",
    );

    let output = Command::new(exe)
        .args(["check", "--error-format=json", file.to_str().unwrap()])
        .output()
        .expect("kobo check json should run");
    let text = combined_output(&output);
    let json_line = text
        .lines()
        .find(|line| line.trim_start().starts_with('{') && line.contains("\"schema_version\""))
        .unwrap_or_else(|| panic!("missing diagnostic JSON line in:\n{text}"));
    let value: serde_json::Value = serde_json::from_str(json_line).unwrap();

    assert!(!output.status.success());
    assert_eq!(value["schema_version"], 1);
    assert!(value["code"].as_str().unwrap().starts_with("K011"));
    assert_eq!(value["severity"], "error");
    assert!(value["primary"]["byte_start"].as_u64().is_some());
    assert!(value["primary"]["line_start"].as_u64().is_some());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn no_color_check_has_no_ansi_but_keeps_source_labels() {
    let exe = env!("CARGO_BIN_EXE_kobo");
    let (root, file) = write_kobo_fixture(
        "kobo-cli-no-color",
        "bad_no_color.kobo",
        "fn main() { let x = }\n",
    );

    let output = Command::new(exe)
        .env("NO_COLOR", "1")
        .args(["check", file.to_str().unwrap()])
        .output()
        .expect("kobo check no-color should run");
    let text = combined_output(&output);

    assert!(!output.status.success());
    assert!(
        !text.contains("\u{1b}["),
        "NO_COLOR output must not contain ANSI escapes"
    );
    assert!(text.contains("-->"), "NO_COLOR must preserve source labels");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn mixed_parse_and_analysis_cli_reports_trustworthy_non_poisoned_diagnostics() {
    let exe = env!("CARGO_BIN_EXE_kobo");
    let (root, file) = write_kobo_fixture(
        "kobo-cli-mixed-recovery",
        "mixed.kobo",
        r#"
fn broken() {
    let x =
}

fn good() {
    let config = String::from("hello");
    process(config);
    log(config);
}

fn process(s: String) {
    println!("process: {}", s);
}

fn log(s: String) {
    println!("log: {}", s);
}
"#,
    );

    let output = Command::new(exe)
        .args([
            "check",
            "--checked",
            "--recover-parse",
            file.to_str().unwrap(),
        ])
        .output()
        .expect("kobo check recover should run");
    let text = combined_output(&output);

    assert!(!output.status.success());
    assert!(
        text.contains("error[K011"),
        "expected parse recovery diagnostic:\n{text}"
    );
    assert!(
        text.contains("K0001"),
        "expected use-after-move diagnostic from good function:\n{text}"
    );

    let _ = fs::remove_dir_all(root);
}
