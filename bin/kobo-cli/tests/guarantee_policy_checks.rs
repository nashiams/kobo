mod cli_test_support;

use cli_test_support::{
    assert_contains, assert_failure, assert_json_has_path, assert_not_contains, assert_success,
    first_json, fixture_text, path_arg, run_kobo, s, unique_symbol, TestProject,
};

#[test]
fn foundation_checks_required_for_runtime_profiles() {
    let repo_root = TestProject::repo_root();

    for target in [
        "crates/compiler/kobo-errors/tests/diagnostic_checks.rs",
        "crates/compiler/kobo-parser/tests/recovery_checks.rs",
        "crates/compiler/kobo-parser/tests/preprocess_source_map_checks.rs",
        "crates/compiler/kobo-driver/tests/parser_recovery_checks.rs",
        "crates/compiler/kobo-driver/tests/query_checks.rs",
        "bin/kobo-cli/tests/diagnostic_cli_checks.rs",
    ] {
        assert!(
            repo_root.join(target).exists(),
            "required foundation check target is missing without an accepted replacement: {target}"
        );
    }

    for (target, replacements) in [
        (
            "bin/kobo-cli/tests/liveness_cli_checks.rs",
            &[
                "bin/kobo-cli/tests/foundation_oracles.rs::slice_04_liveness_debt_distinguishes_unresolved_commit_and_rollback",
                "bin/kobo-cli/tests/edge_oracles.rs::slice_04_edge_escape_and_suppression_reason_are_not_same_diagnostic",
            ][..],
        ),
        (
            "bin/kobo-cli/tests/sim_scout_checks.rs",
            &[
                "bin/kobo-cli/tests/foundation_oracles.rs::slice_05_sim_scout_ranking_changes_with_project_signals",
                "bin/kobo-cli/tests/edge_oracles.rs::slice_05_edge_scout_is_stable_and_does_not_modify_source",
            ][..],
        ),
        (
            "bin/kobo-cli/tests/boundary_policy_checks.rs",
            &[
                "bin/kobo-cli/tests/foundation_oracles.rs::slice_09_boundary_policy_keeps_normal_crates_compatible_and_replay_explicit",
                "bin/kobo-cli/tests/edge_oracles.rs::slice_09_edge_boundary_policy_matrix_is_source_sensitive",
                "bin/kobo-cli/tests/error_policy_checks.rs",
            ][..],
        ),
        (
            "bin/kobo-cli/tests/witness_replay_checks.rs",
            &[
                "bin/kobo-cli/tests/foundation_oracles.rs::slice_07_kwit_replay_validates_schema_without_claiming_full_replay",
                "bin/kobo-cli/tests/edge_oracles.rs::slice_07_edge_kwit_missing_span_fails_unknown_fields_survive",
                "bin/kobo-cli/tests/replay_checks.rs",
            ][..],
        ),
    ] {
        assert_replacement_exists(&repo_root, target, replacements);
    }

    assert_replacement_exists(
        &repo_root,
        "crates/compiler/kobo-lsp/Cargo.toml",
        &[
            "bin/kobo-cli/src/bin/kobo-lsp.rs",
            "bin/kobo-cli/src/commands/lsp_diagnostics.rs",
            "bin/kobo-cli/tests/foundation_oracles.rs::slice_02_lsp_payload_export_matches_cli_json",
            "bin/kobo-cli/tests/lsp_checks.rs",
        ],
    );
}

fn assert_replacement_exists(repo_root: &std::path::Path, target: &str, replacements: &[&str]) {
    if repo_root.join(target).exists() {
        return;
    }

    for replacement in replacements {
        let file_path = replacement.split("::").next().unwrap_or(replacement);
        assert!(
            repo_root.join(file_path).exists(),
            "documented replacement `{replacement}` must exist for missing `{target}`"
        );
    }
}

#[test]
fn guarantee_presets_expand_to_explicit_policy() {
    let project = TestProject::new("guarantee-policy");
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");
    project.copy_fixture("policy/Kobo_checked.toml", "Kobo.toml");

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

    assert_success(&output, "runtime guarantee policy print");
    let json = first_json(&output, "runtime guarantee policy print");
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
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");

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
        release_json["guarantees"], strict_json["guarantees"],
        "`--strict` must be an alias for release/strict guarantees, not another language mode"
    );
}

#[test]
fn strict_downgrade_requires_reason_ledger() {
    let project = TestProject::new("downgrade-reason");
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");
    project.copy_fixture("policy/Kobo_downgrade.toml", "Kobo.toml");

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
fn path_policy_overrides_follow_dynamic_file_globs() {
    let project = TestProject::new("dynamic-path-policy");
    let payment_name = unique_symbol("payment_flow");
    let public_name = unique_symbol("public_flow");
    let basic_source = fixture_text("policy/basic.kobo");
    let payment_file = project.write(&format!("src/payment/{payment_name}.kobo"), &basic_source);
    let public_file = project.write(&format!("src/public/{public_name}.kobo"), &basic_source);
    project.copy_fixture("policy/Kobo_path_glob.toml", "Kobo.toml");

    let payment = run_kobo(
        &[
            s("check"),
            s("--profile"),
            s("release"),
            s("--print-policy=json"),
            path_arg(&payment_file),
        ],
        &project.root,
    );
    let public = run_kobo(
        &[
            s("check"),
            s("--profile"),
            s("release"),
            s("--print-policy=json"),
            path_arg(&public_file),
        ],
        &project.root,
    );

    assert_success(&payment, "payment path policy should be printable");
    assert_success(&public, "public path policy should be printable");
    let payment_json = first_json(&payment, "payment path policy json");
    let public_json = first_json(&public, "public path policy json");
    assert_eq!(
        payment_json["guarantees"]["ownership"], "record",
        "glob override must apply to dynamically named payment file"
    );
    assert_eq!(
        public_json["guarantees"]["ownership"], "strict",
        "default release policy must remain strict outside payment glob"
    );
    assert_ne!(
        payment_json["guarantees"]["ownership"], public_json["guarantees"]["ownership"],
        "policy printer must resolve the actual input path, not emit one canned profile"
    );
}

#[test]
fn release_strict_paths_raise_policy_without_source_mode_identity() {
    let project = TestProject::new("release-strict-paths");
    let payment_name = unique_symbol("payment_strict");
    let public_name = unique_symbol("public_recorded");
    let source = fixture_text("policy/basic.kobo");
    let payment_file = project.write(&format!("src/payment/{payment_name}.kobo"), &source);
    let public_file = project.write(&format!("src/public/{public_name}.kobo"), &source);
    project.write(
        "Kobo.toml",
        r#"[guarantees]
ownership = "record"
liveness = "record"
replay = "record"
boundaries = "record"
errors = "typed"

[ci.release]
deny_new_debt = true
strict_paths = ["src/payment/**"]
deny_downgrade_without_reason = true
"#,
    );

    let payment = run_kobo(
        &[
            s("check"),
            s("--profile"),
            s("release"),
            s("--print-policy=json"),
            path_arg(&payment_file),
        ],
        &project.root,
    );
    let public = run_kobo(
        &[
            s("check"),
            s("--profile"),
            s("release"),
            s("--print-policy=json"),
            path_arg(&public_file),
        ],
        &project.root,
    );

    assert_success(&payment, "strict path policy should print");
    assert_success(&public, "non-strict path policy should print");
    let payment_json = first_json(&payment, "payment strict path policy json");
    let public_json = first_json(&public, "public path policy json");
    assert_eq!(
        payment_json["guarantees"]["ownership"], "strict",
        "release strict_paths should raise matching files back to strict ownership"
    );
    assert_eq!(
        payment_json["guarantees"]["boundaries"], "strict",
        "release strict_paths should raise matching files back to strict boundaries"
    );
    assert_eq!(
        public_json["guarantees"]["ownership"], "record",
        "non-matching files should keep the explicit gradual project policy"
    );
}

#[test]
fn normal_check_enforces_configured_deny_new_debt() {
    let project = TestProject::new("deny-new-debt-normal-check");
    let file = project.main_file(
        r#"#[kobo::relax]
fn relaxed_fn() {
    let value = String::from("ci");
    println!("{}", value);
}

fn main() {
    relaxed_fn();
}
"#,
    );
    project.write(
        "Kobo.toml",
        r#"[ci.release]
deny_new_debt = true
"#,
    );

    let output = run_kobo(&[s("check"), path_arg(&file)], &project.root);

    assert_failure(&output, "configured deny_new_debt must gate normal check");
    let text = output.combined();
    assert_contains(&text, "deny_new_debt", "failure should name the CI gate");
    assert_contains(
        &text,
        "K0026",
        "failure should point at the new debt diagnostic",
    );
}

#[test]
fn normal_build_enforces_configured_deny_new_debt() {
    let project = TestProject::new("deny-new-debt-normal-build");
    let file = project.main_file(
        r#"#[kobo::relax]
fn relaxed_fn() {
    let value = String::from("ci build");
    println!("{}", value);
}

fn main() {
    relaxed_fn();
}
"#,
    );
    project.write(
        "Kobo.toml",
        r#"[ci.release]
deny_new_debt = true
"#,
    );

    let output = run_kobo(&[s("build"), path_arg(&file)], &project.root);

    assert_failure(&output, "configured deny_new_debt must gate normal build");
    let text = output.combined();
    assert_contains(&text, "deny_new_debt", "failure should name the CI gate");
    assert_contains(
        &text,
        "K0026",
        "failure should point at the new debt diagnostic",
    );
}

#[test]
fn normal_check_rejects_configured_downgrade_without_reason() {
    let project = TestProject::new("normal-downgrade-reason");
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");
    project.write(
        "Kobo.toml",
        r#"[guarantees]
ownership = "strict"
liveness = "checked"
replay = "checked"
boundaries = "strict"
errors = "explicit"

[ci.release]
deny_downgrade_without_reason = true

[paths."src/main.kobo"]
ownership = "record"
"#,
    );

    let output = run_kobo(&[s("check"), path_arg(&file)], &project.root);

    assert_failure(
        &output,
        "normal check must reject configured downgrade evidence",
    );
    let text = output.combined();
    assert_contains(&text, "downgrade", "downgrade should be named");
    assert_contains(&text, "reason", "downgrade must ask for a reason");
    assert_contains(&text, "ledger", "downgrade must be recorded as evidence");
}

#[test]
fn normal_check_applies_configured_strict_paths() {
    let project = TestProject::new("normal-strict-paths");
    let source = r#"fn main() {
    let value = String::from("strict path");
    let moved = value;
    println!("{}", value);
}
"#;
    let payment_file = project.write("src/payment/main.kobo", source);
    let public_file = project.write("src/public/main.kobo", source);
    project.write(
        "Kobo.toml",
        r#"[guarantees]
ownership = "record"
liveness = "record"
replay = "record"
boundaries = "record"
errors = "typed"

[ci.release]
deny_new_debt = false
strict_paths = ["src/payment/**"]
"#,
    );

    let payment = run_kobo(&[s("check"), path_arg(&payment_file)], &project.root);
    let public = run_kobo(&[s("check"), path_arg(&public_file)], &project.root);

    assert_failure(&payment, "strict path should raise normal check policy");
    assert_success(
        &public,
        "non-strict path should keep the gradual project policy",
    );
}

#[test]
fn docs_and_cli_do_not_expose_script_strict_as_language_identities() {
    let project = TestProject::new("no-mode-identity");
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");
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
        "public output must not describe source as Script code or Strict code:\n{text}"
    );
}

#[test]
fn k010x_explain_codes_use_runtime_meanings() {
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
        assert_contains(&text, meaning, "explain must use guarantee-policy meaning");
        assert!(
            !text.contains("parser recovery") && !text.contains("syntax error recovered"),
            "{code} must not retain old parser-recovery meaning:\n{text}"
        );
    }
}

#[test]
fn public_outputs_do_not_report_closed_audit_findings() {
    let project = TestProject::new("public-audit-clean");
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");
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

    assert_success(&output, "policy output should be printable");
    let text = output.combined();
    for stale_finding in [
        "Replay `--error-format` is ignored",
        "`kobo inspect --sim` still reports",
        "No full expanded policy snapshot",
        "No `not_replayable` replay guarantee variant",
        "Boundary assumptions exist, but not as a full audited assumption ledger",
    ] {
        assert_not_contains(
            &text,
            stale_finding,
            "public output must not report closed implementation-audit findings",
        );
    }
}
