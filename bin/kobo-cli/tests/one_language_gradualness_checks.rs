mod cli_common;

use cli_common::{
    assert_contains, assert_not_contains, assert_success, json_lines, path_arg, run_kobo, s,
    TestProject,
};

#[test]
fn public_help_describes_profiles_not_language_modes() {
    let project = TestProject::new("runtime-help-profile-language");

    for command in ["check", "run", "inspect", "build"] {
        let output = run_kobo(&[s(command), s("--help")], &project.root);
        assert_success(&output, &format!("{command} --help"));
        let text = output.combined().to_lowercase();

        for banned in [
            "script mode",
            "checked mode",
            "strict mode",
            "script/checked mode",
            "release/strict guarantee mode",
        ] {
            assert_not_contains(
                &text,
                banned,
                &format!("{command} help must not expose old mode identity"),
            );
        }
    }

    let build_help = run_kobo(&[s("build"), s("--help")], &project.root);
    let build_text = build_help.combined().to_lowercase();
    assert_contains(
        &build_text,
        "compatibility alias",
        "--strict must be described as compatibility only",
    );
    assert_contains(
        &build_text,
        "--profile release",
        "--strict help must name the release profile replacement",
    );
}

#[test]
fn legacy_file_mode_directive_emits_profile_migration_diagnostic() {
    let project = TestProject::new("runtime-legacy-file-mode");
    let file = project.main_file(
        r#"//! kobo:mode = strict
fn main() {
    println!("legacy directive");
}
"#,
    );

    let output = run_kobo(
        &[s("check"), s("--error-format=json"), path_arg(&file)],
        &project.root,
    );

    assert_success(
        &output,
        "legacy kobo:mode directive remains compatibility path",
    );
    let diagnostics = json_lines(&output);
    assert!(
        diagnostics
            .iter()
            .any(|value| value["code"].as_str() == Some("K0096")),
        "legacy mode directive must be a structured diagnostic, got:\n{}",
        output.combined()
    );
    let text = output.combined();
    assert_contains(
        &text,
        "legacy",
        "diagnostic must mark the directive as legacy",
    );
    assert_contains(&text, "kobo:mode", "diagnostic must name the old directive");
    assert_contains(
        &text,
        "--profile release",
        "diagnostic must provide the equivalent profile",
    );
    assert_contains(
        &text,
        "guarantee policy",
        "diagnostic must explain the new policy model",
    );
}

#[test]
fn accepted_program_has_same_runtime_output_across_profiles() {
    let project = TestProject::new("runtime-runtime-profile-parity");
    let file = project.main_file(
        r#"fn main() {
    println!("same runtime meaning");
}
"#,
    );
    let mut outputs = Vec::new();

    for profile in ["dev", "checked", "release"] {
        let output = run_kobo(
            &[s("run"), s("--profile"), s(profile), path_arg(&file)],
            &project.root,
        );
        assert_success(&output, &format!("run --profile {profile}"));
        outputs.push((profile, output.stdout));
    }

    let expected = outputs[0].1.clone();
    for (profile, stdout) in outputs {
        assert_eq!(
            stdout, expected,
            "accepted code must keep the same ordinary runtime behavior under {profile}"
        );
    }
}
