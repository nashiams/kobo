mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo, s, unique_symbol,
    TestProject,
};

const ERROR_SOURCE: &str = r#"
fn load_user() -> Result<String, std::io::Error> {
    let value = std::fs::read_to_string("user.txt")?;
    Ok(value)
}

fn main() {
    let _ = load_user();
}
"#;

#[test]
fn guarantee_error_strategy_differs_by_policy_without_runtime_result_change() {
    let project = TestProject::new("error-policy");
    let file = project.main_file(ERROR_SOURCE);
    project.write(
        "Kobo.toml",
        r#"
[profiles.dev.guarantees]
errors = "ergonomic"

[profiles.checked.guarantees]
errors = "typed"

[profiles.release.guarantees]
errors = "explicit"
"#,
    );

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
        &release.combined(),
        "explicit",
        "release should require explicit error policy evidence",
    );
}

#[test]
fn external_crate_on_replay_path_emits_k0107_policy_choices() {
    let project = TestProject::new("boundary-policy");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
async fn replay_http() {
    let _ = reqwest::Client::new();
}
"#,
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
    let file = project.main_file(&format!(
        r#"
#[kobo::scenario(profile = "async")]
async fn replay_dynamic_boundary() {{
    let _ = {crate_name}::Client::new();
}}
"#
    ));

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
fn external_crate_normal_path_builds_without_sim_adapter() {
    let project = TestProject::new("normal-crate-compat");
    let file = project.main_file(
        r#"
fn main() {
    let _ = reqwest::Client::new();
}
"#,
    );

    let output = run_kobo(
        &[s("build"), s("--profile"), s("dev"), path_arg(&file)],
        &project.root,
    );

    assert_success(
        &output,
        "normal Rust crate usage must not require simulation adapter",
    );
}
