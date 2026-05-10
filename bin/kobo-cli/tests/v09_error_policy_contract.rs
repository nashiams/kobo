mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo, s, unique_symbol,
    TestProject,
};

#[test]
fn guarantee_error_strategy_differs_by_policy_without_runtime_result_change() {
    let project = TestProject::new("error-policy");
    let file = project.copy_fixture("errors/error_source.kobo", "src/main.kobo");
    project.copy_fixture("errors/Kobo_error_policy.toml", "Kobo.toml");

    let dev = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("dev"),
            s("--emit-rust"),
            path_arg(&file),
        ],
        &project.root,
    );
    let dev_generated = project.read("src/main.rs");
    let checked = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("checked"),
            s("--emit-rust"),
            path_arg(&file),
        ],
        &project.root,
    );
    let release = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--emit-rust"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&dev, "dev error strategy should compile");
    assert_success(&checked, "checked error strategy should compile");
    assert_success(&release, "release error strategy should compile");
    assert_contains(
        &dev.combined(),
        "Box<dyn",
        "dev should use ergonomic boxed error shape",
    );
    assert_contains(
        &checked.combined(),
        "enum",
        "checked should generate typed enum shape",
    );
    assert_contains(
        &checked.combined(),
        "ReadToString",
        "checked typed error policy must derive a concrete variant from the ? site",
    );
    assert_contains(
        &checked.combined(),
        "map_err(KoboTypedError::ReadToString)",
        "checked typed error policy must route the ? site through the generated variant",
    );
    assert_contains(
        &release.combined(),
        "explicit",
        "release should require explicit error policy evidence",
    );
    assert_contains(
        &release.combined(),
        "error_site line",
        "release explicit policy must record site-level evidence",
    );
    assert_contains(
        &dev_generated,
        "Box<dyn",
        "generated Rust file must contain the selected dev error policy, not only stdout",
    );
}

#[test]
fn external_crate_on_replay_path_emits_k0107_policy_choices() {
    let project = TestProject::new("boundary-policy");
    let file = project.copy_fixture("errors/boundary_replay_http.kobo", "src/main.kobo");

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    let text = output.combined();
    assert_failure(
        &output,
        "external replay boundary should fail until policy is chosen",
    );
    assert_contains(&text, "K0107", "external replay boundary must emit K0107");
    for choice in ["model", "record", "stub", "outside", "opaque", "debt"] {
        assert_contains(
            &text,
            choice,
            "K0107 must expose every boundary policy choice",
        );
    }
}

#[test]
fn boundary_diagnostic_names_dynamic_external_crate() {
    let project = TestProject::new("boundary-dynamic-crate");
    let crate_name = unique_symbol("external_service");
    let file = project.copy_fixture_template(
        "errors/boundary_dynamic.template.kobo",
        "src/main.kobo",
        &[("__CRATE__", &crate_name)],
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    let text = output.combined();
    assert_failure(&output, "dynamic external boundary should require policy");
    assert_contains(&text, "K0107", "dynamic external boundary must emit K0107");
    assert_contains(
        &text,
        &crate_name,
        "K0107 must identify the actual external boundary, not a canned crate",
    );
}

#[test]
fn boundary_failure_witness_records_policy_choices_and_dynamic_crate() {
    let project = TestProject::new("boundary-witness-policy");
    let crate_name = unique_symbol("external_service");
    let file = project.copy_fixture_template(
        "errors/boundary_dynamic.template.kobo",
        "src/main.kobo",
        &[("__CRATE__", &crate_name)],
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "dynamic external boundary should emit a witness");
    let witnesses = project.find_files_with_ext("kwit");
    assert!(
        !witnesses.is_empty(),
        "boundary failure should create a .kwit witness"
    );
    let witness = std::fs::read_to_string(&witnesses[0]).expect("witness should be readable");
    assert_contains(
        &witness,
        &crate_name,
        "witness boundary metadata must name the actual external crate",
    );
    assert_contains(
        &witness,
        "unselected",
        "witness must record that no boundary policy has been selected",
    );
    assert_contains(
        &witness,
        "partial",
        "opaque boundary witness must not claim exact deterministic replay",
    );
}

#[test]
fn external_crate_normal_path_builds_without_sim_adapter() {
    let project = TestProject::new("normal-crate-compat");
    let file = project.copy_fixture("errors/boundary_normal_http.kobo", "src/main.kobo");

    let output = run_kobo(
        &[s("build"), s("--profile"), s("dev"), path_arg(&file)],
        &project.root,
    );

    assert_success(
        &output,
        "normal Rust crate usage must not require simulation adapter",
    );
}
