mod v09_common;

use std::{fs, path::Path};

use v09_common::{
    assert_contains, assert_failure, assert_json_has_path, assert_success, first_json,
    fixture_text, path_arg, run_kobo, s, unique_symbol, TestProject,
};

#[test]
fn v085_foundation_required_for_v09() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.join("..").join("..");
    let note_path = repo_root
        .join(".claude/prompt/roadmap/v0.9/implementation_notes/phase_00_v085_foundation_lock.md");
    let note = fs::read_to_string(&note_path).unwrap_or_else(|error| {
        panic!(
            "phase 00 implementation note should be readable at {}: {error}",
            note_path.display()
        )
    });

    for target in [
        "crates/compiler/kobo-errors/tests/diagnostic_contract.rs",
        "crates/compiler/kobo-parser/tests/recovery_contract.rs",
        "crates/compiler/kobo-parser/tests/preprocess_source_map_contract.rs",
        "crates/compiler/kobo-driver/tests/parser_recovery_contract.rs",
        "crates/compiler/kobo-driver/tests/query_contract.rs",
        "bin/kobo-cli/tests/diagnostic_cli_contract.rs",
    ] {
        assert!(
            repo_root.join(target).exists(),
            "required v0.8.5 contract target is missing without an accepted replacement: {target}"
        );
    }

    for (target, replacements) in [
        (
            "bin/kobo-cli/tests/liveness_cli_contract.rs",
            &[
                "bin/kobo-cli/tests/v085_phase_oracles.rs::phase_04_liveness_debt_distinguishes_unresolved_commit_and_rollback",
                "bin/kobo-cli/tests/v085_edge_oracles.rs::phase_04_edge_escape_and_suppression_reason_are_not_same_diagnostic",
            ][..],
        ),
        (
            "bin/kobo-cli/tests/sim_scout_contract.rs",
            &[
                "bin/kobo-cli/tests/v085_phase_oracles.rs::phase_05_sim_scout_ranking_changes_with_project_signals",
                "bin/kobo-cli/tests/v085_edge_oracles.rs::phase_05_edge_scout_is_stable_and_does_not_modify_source",
            ][..],
        ),
        (
            "bin/kobo-cli/tests/boundary_policy_contract.rs",
            &[
                "bin/kobo-cli/tests/v085_phase_oracles.rs::phase_09_boundary_policy_keeps_normal_crates_compatible_and_replay_explicit",
                "bin/kobo-cli/tests/v085_edge_oracles.rs::phase_09_edge_boundary_policy_matrix_is_source_sensitive",
                "bin/kobo-cli/tests/v09_error_policy_contract.rs",
            ][..],
        ),
        (
            "bin/kobo-cli/tests/witness_replay_contract.rs",
            &[
                "bin/kobo-cli/tests/v085_phase_oracles.rs::phase_07_kwit_replay_validates_schema_without_claiming_full_replay",
                "bin/kobo-cli/tests/v085_edge_oracles.rs::phase_07_edge_kwit_missing_span_fails_unknown_fields_survive",
                "bin/kobo-cli/tests/v09_replay_contract.rs",
            ][..],
        ),
    ] {
        assert_documented_replacement(&repo_root, &note, target, replacements);
    }

    assert_documented_replacement(
        &repo_root,
        &note,
        "crates/compiler/kobo-lsp/Cargo.toml",
        &[
            "bin/kobo-cli/src/bin/kobo-lsp.rs",
            "bin/kobo-cli/src/commands/lsp_diagnostics.rs",
            "bin/kobo-cli/tests/v085_phase_oracles.rs::phase_02_lsp_payload_export_matches_cli_json",
            "bin/kobo-cli/tests/v09_lsp_contract.rs",
        ],
    );
}

fn assert_documented_replacement(
    repo_root: &Path,
    note: &str,
    target: &str,
    replacements: &[&str],
) {
    if repo_root.join(target).exists() {
        return;
    }

    assert!(
        note.contains(target),
        "missing contract target `{target}` must be named in the phase 00 replacement map"
    );
    for replacement in replacements {
        let file_path = replacement.split("::").next().unwrap_or(replacement);
        assert!(
            repo_root.join(file_path).exists(),
            "documented replacement `{replacement}` must exist for missing `{target}`"
        );
        assert!(
            note.contains(replacement),
            "phase 00 note must document exact replacement `{replacement}` for missing `{target}`"
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
