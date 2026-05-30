use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture_dir(name: &str) -> PathBuf {
    let root = workspace_root();
    let dir = root
        .join("target")
        .join("ownership-debt-pipeline")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create fixture dir");
    fs::write(dir.join("Kobo.toml"), "# local fixture policy\n")
        .expect("failed to write local Kobo.toml");
    dir
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("kobo-cli is expected to live under bin/kobo-cli")
        .to_path_buf()
}

fn write_case(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(format!("{name}.kobo"));
    fs::write(&path, source).expect("failed to write .kobo fixture");
    path
}

fn kobo(args: &[&str], file: &Path) -> Output {
    let exe = env!("CARGO_BIN_EXE_kobo");
    Command::new(exe)
        .args(args)
        .arg(file)
        .output()
        .expect("failed to run kobo")
}

fn kobo_with_env(args: &[&str], file: &Path, key: &str, value: &str) -> Output {
    let exe = env!("CARGO_BIN_EXE_kobo");
    Command::new(exe)
        .args(args)
        .arg(file)
        .env(key, value)
        .output()
        .expect("failed to run kobo")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn json_value(output: &Output) -> serde_json::Value {
    serde_json::from_str(&combined(output)).expect("command should emit JSON")
}

fn ownership_debt_code_count(value: &serde_json::Value, code: &str) -> usize {
    ownership_debt_entries(value)
        .iter()
        .filter(|entry| entry["code"].as_str() == Some(code))
        .count()
}

fn ownership_debt_entries(value: &serde_json::Value) -> &[serde_json::Value] {
    value["ownership_debt"]
        .as_array()
        .expect("ownership_debt should be an array")
}

fn assert_ownership_debt_record(value: &serde_json::Value, code: &str, kind: &str) {
    let matching = ownership_debt_entries(value)
        .iter()
        .filter(|entry| entry["code"].as_str() == Some(code))
        .collect::<Vec<_>>();

    assert!(
        !matching.is_empty(),
        "ownership_debt should include structured {code}: {value:#}"
    );
    for entry in matching {
        let expected_severity = if code == "K0099" { "error" } else { "warning" };
        assert_eq!(
            entry["severity"], expected_severity,
            "{code} severity should be stable"
        );
        assert_eq!(entry["kind"], kind, "{code} kind should be stable");
        assert!(
            entry["source_span"].is_object(),
            "{code} should preserve a source span: {entry:#}"
        );
        assert!(
            entry["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "{code} should preserve a message: {entry:#}"
        );
        assert!(
            entry["hint"]
                .as_str()
                .is_some_and(|hint| hint.starts_with("ownership debt:")),
            "{code} should preserve an ownership-debt hint: {entry:#}"
        );
    }
}

fn expected_ownership_kind(code: &str) -> &'static str {
    match code {
        "K0001" => "use_after_move",
        "K0002" => "borrow_conflict",
        "K0032" => "rewrite_required",
        "K0099" => "rustc_escape",
        other => panic!("unexpected ownership debt code {other}"),
    }
}

fn cases() -> BTreeMap<&'static str, &'static str> {
    [
        (
            "use_after_move_into_function",
            r#"
fn main() {
    let message = String::from("paid");
    consume(message);
    consume(message);
}

fn consume(s: String) {
    println!("{}", s);
}
"#,
        ),
        (
            "use_after_move_macro",
            r#"
fn main() {
    let message = String::from("paid");
    let moved = message;
    println!("{}", message);
    println!("{}", moved);
}
"#,
        ),
        (
            "mutate_while_shared_borrow_lives",
            r#"
fn main() {
    let mut data = vec![1, 2, 3];
    let view = &data;
    data.push(4);
    println!("{:?}", view);
}
"#,
        ),
        (
            "double_mut_borrow",
            r#"
fn main() {
    let mut data = vec![1, 2, 3];
    let first = &mut data;
    let second = &mut data;
    println!("{:?} {:?}", first, second);
}
"#,
        ),
        (
            "move_while_borrowed",
            r#"
fn main() {
    let message = String::from("paid");
    let view = &message;
    let moved = message;
    println!("{} {}", view, moved);
}
"#,
        ),
    ]
    .into_iter()
    .collect()
}

const OWNERSHIP_DEBT_CASES: &[(&str, &str, &[&str])] = &[
    (
        "use_after_move_fn",
        "use_after_move_into_function",
        &["K0001"],
    ),
    ("use_after_move_macro", "use_after_move_macro", &["K0001"]),
    (
        "mut_borrow_conflict",
        "mutate_while_shared_borrow_lives",
        &["K0002"],
    ),
    ("double_mut_borrow", "double_mut_borrow", &["K0002"]),
    ("move_while_borrowed", "move_while_borrowed", &["K0032"]),
];

#[test]
fn checked_profile_is_not_silently_downgraded_by_root_guarantees() {
    let dir = fixture_dir("profile-floor");
    fs::remove_file(dir.join("Kobo.toml")).expect("remove local policy so root policy is used");
    let file = write_case(
        &dir,
        "use_after_move",
        cases()["use_after_move_into_function"],
    );

    let output = kobo(
        &[
            "check",
            "--checked",
            "--print-policy=json",
            "--error-format=json",
        ],
        &file,
    );
    let text = combined(&output);

    assert!(
        text.contains("\"ownership\":\"checked\""),
        "explicit --checked must keep ownership checked; output:\n{text}"
    );
}

#[test]
fn ownership_diagnostics_are_visible_in_debt_report() {
    for (name, source_key, expected_codes) in OWNERSHIP_DEBT_CASES {
        let dir = fixture_dir(&format!("debt-{name}"));
        let file = write_case(&dir, name, cases()[source_key]);
        let output = kobo(&["debt", "--json"], &file);
        let text = combined(&output);
        let value = json_value(&output);

        assert!(
            output.status.success(),
            "debt command failed for {name}:\n{text}"
        );
        for code in *expected_codes {
            assert_ownership_debt_record(&value, code, expected_ownership_kind(code));
        }
    }
}

#[test]
fn move_while_borrowed_debt_is_not_duplicated() {
    let dir = fixture_dir("debt-k0032-dedup");
    let file = write_case(&dir, "move_while_borrowed", cases()["move_while_borrowed"]);
    let output = kobo(&["debt", "--json"], &file);
    let value = json_value(&output);

    assert_eq!(
        ownership_debt_code_count(&value, "K0032"),
        1,
        "move-while-borrowed should produce exactly one K0032 debt record:\n{}",
        combined(&output)
    );
}

#[test]
fn strict_run_stops_before_rustc_for_known_ownership_debt() {
    let dir = fixture_dir("strict-run-known-debt");
    let file = write_case(
        &dir,
        "use_after_move_fn",
        cases()["use_after_move_into_function"],
    );

    let output = kobo(&["run", "--strict"], &file);
    let text = combined(&output);

    assert!(!output.status.success(), "strict run should fail:\n{text}");
    assert!(text.contains("error[K0001]"), "expected K0001:\n{text}");
    assert!(
        !text.contains("error[K0099]"),
        "known ownership debt must not fall through to rustc K0099:\n{text}"
    );
}

#[test]
fn rustc_ownership_escapes_are_classified_as_ownership_debt() {
    let cases = [
        (
            "move_out_of_borrow",
            r#"
fn main() {
    let message = String::from("paid");
    let view = &message;
    let moved = *view;
    println!("{}", moved);
}
"#,
            "E0507",
        ),
        (
            "return_local_ref",
            r#"
fn make_ref() -> &'static String {
    let message = String::from("paid");
    &message
}

fn main() {
    println!("{}", make_ref());
}
"#,
            "E0515",
        ),
    ];

    for (name, source, rustc_code) in cases {
        let dir = fixture_dir(&format!("rustc-escape-{name}"));
        let file = write_case(&dir, name, source);
        let output = kobo(&["run"], &file);
        let text = combined(&output);

        assert!(!output.status.success(), "{name} should fail:\n{text}");
        assert!(
            text.contains("K0099"),
            "{name} should still mention K0099:\n{text}"
        );
        assert!(
            text.contains(rustc_code),
            "{name} should preserve rustc code {rustc_code}:\n{text}"
        );
        assert!(
            text.contains("ownership debt") || text.contains("ownership error escaped"),
            "{name} should explain this as ownership debt, not generic compiler failure:\n{text}"
        );
        assert!(
            !text.contains("aborting due to"),
            "{name} should suppress rustc follow-up errors:\n{text}"
        );
        assert!(
            !text.contains("kobo doctor"),
            "{name} is a local ownership escape and should not suggest doctor:\n{text}"
        );
    }
}

#[test]
fn all_rust_ownership_shapes_are_classified_by_cli_run() {
    let cases = [
        (
            "e0382_use_after_move",
            cases()["use_after_move_into_function"],
            "K0001",
        ),
        (
            "e0499_double_mut_borrow",
            cases()["double_mut_borrow"],
            "K0002",
        ),
        (
            "e0502_shared_then_mut",
            cases()["mutate_while_shared_borrow_lives"],
            "K0002",
        ),
        (
            "e0505_move_while_borrowed",
            cases()["move_while_borrowed"],
            "K0032",
        ),
        (
            "e0507_move_out_of_borrow",
            r#"
fn main() {
    let message = String::from("paid");
    let view = &message;
    let moved = *view;
    println!("{}", moved);
}
"#,
            "K0099",
        ),
        (
            "e0515_return_local_ref",
            r#"
fn make_ref() -> &'static String {
    let message = String::from("paid");
    &message
}

fn main() {
    println!("{}", make_ref());
}
"#,
            "K0099",
        ),
    ];

    for (name, source, expected_code) in cases {
        let dir = fixture_dir(&format!("run-{name}"));
        let file = write_case(&dir, name, source);
        let output = kobo(&["run"], &file);
        let text = combined(&output);

        if expected_code == "K0032" {
            assert!(
                output.status.success(),
                "{name} may run only as a safe rewrite with visible debt:\n{text}"
            );
        } else {
            assert!(!output.status.success(), "{name} should fail:\n{text}");
        }
        assert!(
            text.contains(expected_code),
            "{name} should be classified as {expected_code}:\n{text}"
        );
        assert!(
            text.contains("ownership") || text.contains("borrow"),
            "{name} should explain the ownership shape:\n{text}"
        );
    }
}

#[test]
fn debt_json_records_rustc_ownership_escape() {
    let dir = fixture_dir("debt-rustc-escape");
    let file = write_case(
        &dir,
        "move_out_of_borrow",
        r#"
fn main() {
    let message = String::from("paid");
    let view = &message;
    let moved = *view;
    println!("{}", moved);
}
"#,
    );
    let output = kobo(&["debt", "--json"], &file);
    let text = combined(&output);
    let value = json_value(&output);

    assert!(
        output.status.success(),
        "debt should report rustc escapes without failing:\n{text}"
    );
    assert_eq!(
        ownership_debt_code_count(&value, "K0099"),
        1,
        "rustc ownership escape should enter ownership_debt:\n{text}"
    );
    assert_ownership_debt_record(&value, "K0099", "rustc_escape");
    assert!(
        text.contains("rustc_escape") && text.contains("E0507"),
        "debt JSON should preserve escape kind and rustc code:\n{text}"
    );
}

#[test]
fn double_mut_borrow_does_not_become_runtime_refcell_panic() {
    let dir = fixture_dir("double-mut-borrow-no-runtime-panic");
    let file = write_case(&dir, "double_mut_borrow", cases()["double_mut_borrow"]);

    let output = kobo(&["run", "--checked"], &file);
    let text = combined(&output);

    assert!(!output.status.success(), "checked run should stop:\n{text}");
    assert!(text.contains("K0002"), "expected borrow debt:\n{text}");
    assert!(
        !text.contains("RefCell already borrowed"),
        "Kobo must not allow known borrow debt to become runtime RefCell panic:\n{text}"
    );
}

#[test]
fn mixed_known_and_rustc_escape_debt_are_both_recorded() {
    let dir = fixture_dir("debt-mixed-known-and-rustc-escape");
    let file = write_case(
        &dir,
        "mixed",
        r#"
fn main() {
    let message = String::from("paid");
    consume(message);
    consume(message);

    let other = String::from("held");
    let view = &other;
    let moved = *view;
    println!("{}", moved);
}

fn consume(s: String) {
    println!("{}", s);
}
"#,
    );

    let output = kobo(&["debt", "--json"], &file);
    let value = json_value(&output);

    assert_ownership_debt_record(&value, "K0001", "use_after_move");
    assert_ownership_debt_record(&value, "K0099", "rustc_escape");
}

#[test]
fn debt_probe_does_not_write_generated_files_next_to_source() {
    let dir = fixture_dir("debt-rustc-escape-artifacts");
    let file = write_case(
        &dir,
        "move_out_of_borrow",
        r#"
fn main() {
    let message = String::from("paid");
    let view = &message;
    let moved = *view;
    println!("{}", moved);
}
"#,
    );

    let output = kobo(&["debt", "--json"], &file);
    assert!(
        output.status.success(),
        "debt should succeed without source-adjacent artifacts:\n{}",
        combined(&output)
    );
    assert!(
        !file.with_extension("rs").exists(),
        "debt must not write generated Rust beside the source"
    );
    assert!(
        !file.with_extension("kobo.map").exists(),
        "debt must not write a source map beside the source"
    );
}

#[test]
fn release_build_reports_move_while_borrowed_as_k0032() {
    let dir = fixture_dir("build-k0032-message");
    let file = write_case(&dir, "move_while_borrowed", cases()["move_while_borrowed"]);
    let output = kobo(&["build", "--profile", "release"], &file);
    let text = combined(&output);

    assert!(
        !output.status.success(),
        "release build should fail:\n{text}"
    );
    assert!(
        text.contains("K0032 ownership debt"),
        "release build should name the K0032 debt:\n{text}"
    );
    assert!(
        !text.contains("K0001 ownership debt"),
        "release build must not mislabel K0032 as K0001:\n{text}"
    );
}

#[test]
fn debt_human_output_uses_terse_warning_lines() {
    let dir = fixture_dir("debt-terse-warning-lines");
    let file = write_case(
        &dir,
        "use_after_move",
        cases()["use_after_move_into_function"],
    );

    let output = kobo(&["debt"], &file);
    let text = combined(&output);

    assert!(
        text.contains("warning[K0001]"),
        "debt output should use warning severity:\n{text}"
    );
    assert!(
        text.contains("use_after_move.kobo:"),
        "debt output should include file/line/column:\n{text}"
    );
    assert!(
        text.contains("  hint: ownership debt:"),
        "debt output should keep the short hint line:\n{text}"
    );
    assert!(
        !text.contains("What Kobo found:"),
        "debt output should stay terse; full cards belong to kobo check/explain:\n{text}"
    );
}

#[test]
fn debt_warning_color_is_yellow_when_forced() {
    let dir = fixture_dir("debt-warning-color");
    let file = write_case(
        &dir,
        "borrow_conflict",
        cases()["mutate_while_shared_borrow_lives"],
    );

    let colored = kobo(&["debt", "--color=always"], &file);
    let colored_text = combined(&colored);
    assert!(
        colored_text.contains("\x1b[1;33mwarning[K0002]"),
        "forced color should make warning debt yellow:\n{colored_text}"
    );
    assert!(
        colored_text.contains("\x1b[2;33m  hint: ownership debt:"),
        "forced color should make the hint dim yellow:\n{colored_text}"
    );

    let plain = kobo(&["debt", "--color=never"], &file);
    let plain_text = combined(&plain);
    assert!(
        !plain_text.contains("\x1b["),
        "--color=never must suppress ANSI escapes:\n{plain_text}"
    );
    assert!(
        plain_text.contains("warning[K0002]"),
        "plain output must keep warning identity:\n{plain_text}"
    );
    assert!(
        plain_text.contains("  hint: ownership debt:"),
        "plain output must keep debt hint text:\n{plain_text}"
    );

    let no_color = kobo_with_env(&["debt", "--color=always"], &file, "NO_COLOR", "1");
    let no_color_text = combined(&no_color);
    assert!(
        !no_color_text.contains("\x1b["),
        "NO_COLOR must override forced color:\n{no_color_text}"
    );
}

#[test]
fn debt_structural_hint_is_kept_and_colored() {
    let file = workspace_root().join("tests/ui/K0080_P1_node.kobo");

    let plain = kobo(&["debt", "--color=never"], &file);
    let plain_text = combined(&plain);
    assert!(
        plain_text.contains("consider Weak<T> for one direction or arena allocation"),
        "structural debt must keep the migration hint:\n{plain_text}"
    );

    let colored = kobo(&["debt", "--color=always"], &file);
    let colored_text = combined(&colored);
    assert!(
        colored_text.contains("\x1b[1;33m  note[K0080-P1]"),
        "structural warning headline should be yellow:\n{colored_text}"
    );
    assert!(
        colored_text.contains(
            "\x1b[2;33m  = migration: consider Weak<T> for one direction or arena allocation"
        ),
        "structural hint should be dim yellow:\n{colored_text}"
    );
}
