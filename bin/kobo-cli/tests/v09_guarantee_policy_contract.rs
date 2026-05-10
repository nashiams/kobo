mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_json_has_path, assert_success, first_json, path_arg,
    run_kobo, s, TestProject,
};

const BASIC_SOURCE: &str = r#"
fn main() {
    let value = String::from("v09");
    println!("{}", value);
}
"#;

#[test]
fn guarantee_presets_expand_to_explicit_policy() {
    let project = TestProject::new("guarantee-policy");
    let file = project.main_file(BASIC_SOURCE);
    project.write(
        "Kobo.toml",
        r#"
[guarantees]
ownership = "record"
liveness = "checked"
replay = "checked"
boundaries = "record"
errors = "typed"

[ci.release]
deny_new_debt = true
strict_paths = ["src/payment/**", "src/auth/**"]
deny_downgrade_without_reason = true
"#,
    );

    let output = run_kobo(
        &[
            s("check"),
            s("--profile"),
            s("checked"),
            s("--print-policy=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "v0.9 guarantee policy print");
    let json = first_json(&output, "v0.9 guarantee policy print");
    for path in [
        ["profile"].as_slice(),
        ["guarantees", "ownership"].as_slice(),
        ["guarantees", "liveness"].as_slice(),
        ["guarantees", "replay"].as_slice(),
        ["guarantees", "boundaries"].as_slice(),
        ["guarantees", "errors"].as_slice(),
        ["ci", "release", "deny_new_debt"].as_slice(),
    ] {
        assert_json_has_path(&json, path, "preset must expand to explicit policy");
    }
    assert_eq!(json["profile"], "checked");
    assert_eq!(json["guarantees"]["replay"], "checked");
}

#[test]
fn strict_alias_matches_release_policy() {
    let project = TestProject::new("strict-alias");
    let file = project.main_file(BASIC_SOURCE);

    let release = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--print-policy=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let strict = run_kobo(
        &[
            s("build"),
            s("--strict"),
            s("--print-policy=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&release, "release profile should build");
    assert_success(&strict, "strict alias should build");
    let release_json = first_json(&release, "release policy json");
    let strict_json = first_json(&strict, "strict alias policy json");
    assert_eq!(
        release_json["guarantees"],
        strict_json["guarantees"],
        "`--strict` must be an alias for release/strict guarantees, not another language mode"
    );
}

#[test]
fn strict_downgrade_requires_reason_ledger() {
    let project = TestProject::new("downgrade-reason");
    let file = project.main_file(BASIC_SOURCE);
    project.write(
        "Kobo.toml",
        r#"
[guarantees]
ownership = "strict"
liveness = "checked"
replay = "checked"
boundaries = "strict"
errors = "explicit"

[paths."src/main.kobo"]
ownership = "record"
"#,
    );

    let output = run_kobo(
        &[
            s("check"),
            s("--profile"),
            s("release"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "silent downgrade must be rejected");
    let text = output.combined();
    assert_contains(&text, "downgrade", "downgrade should be named");
    assert_contains(&text, "reason", "downgrade must ask for a reason");
    assert_contains(&text, "ledger", "downgrade must be recorded as evidence");
}

#[test]
fn docs_and_cli_do_not_expose_script_strict_as_language_identities() {
    let project = TestProject::new("no-mode-identity");
    let file = project.main_file(BASIC_SOURCE);
    let output = run_kobo(
        &[s("check"), s("--profile"), s("dev"), path_arg(&file)],
        &project.root,
    );

    assert_success(&output, "dev profile check should succeed");
    let text = output.combined();
    assert_contains(&text, "profile", "output should mention guarantee profile");
    assert_contains(&text, "guarantee", "output should mention guarantee policy");
    assert!(
        !text.contains("Script code") && !text.contains("Strict code"),
        "v0.9 must not describe source as Script code or Strict code:\n{text}"
    );
}

#[test]
fn k010x_explain_codes_use_v09_meanings() {
    let project = TestProject::new("k010x-explain");

    for (code, meaning) in [
        ("K0100", "liveness"),
        ("K0102", "nondeterminism"),
        ("K0103", "uncontrolled"),
        ("K0104", "diverged"),
        ("K0105", "budget"),
        ("K0107", "boundary"),
    ] {
        let output = run_kobo(&[s("explain"), s(code)], &project.root);
        assert_success(&output, "K010x explain must exist");
        let text = output.combined().to_lowercase();
        assert_contains(&text, &code.to_lowercase(), "explain must name code");
        assert_contains(&text, meaning, "explain must use v0.9 meaning");
        assert!(
            !text.contains("parser recovery") && !text.contains("syntax error recovered"),
            "{code} must not retain old parser-recovery meaning:\n{text}"
        );
    }
}
