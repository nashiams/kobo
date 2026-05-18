mod v09_common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg,
    run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

fn write_path_dependency(project: &TestProject, name: &str, feature_name: Option<&str>) {
    let mut cargo = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#
    );
    if let Some(feature_name) = feature_name {
        cargo.push_str(&format!(
            r#"
[features]
{feature_name} = []
"#
        ));
    }
    project.write(&format!("{name}/Cargo.toml"), &cargo);
    project.write(
        &format!("{name}/src/lib.rs"),
        r#"
pub fn helper_value() -> &'static str {
    "cargo path dependency"
}

#[cfg(feature = "fancy")]
pub fn feature_marker() -> &'static str {
    "fancy feature"
}
"#,
    );
}

fn write_cargo_manifest(project: &TestProject, dependency_name: &str, dependency_path: &str) {
    project.write("src/main.rs", "fn main() {}\n");
    project.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "v11-cargo-compat"
version = "0.1.0"
edition = "2021"

[dependencies]
{dependency_name} = {{ path = "{dependency_path}", features = ["fancy"] }}
"#
        ),
    );
}

#[test]
fn normal_path_dependency_builds_without_kobo_metadata() {
    let project = TestProject::new("v11-cargo-path-dep");
    write_path_dependency(&project, "helper_dep", Some("fancy"));
    write_cargo_manifest(&project, "helper_dep", "helper_dep");
    project.main_file(
        r#"
use helper_dep::{feature_marker, helper_value};

fn main() {
    let _value = helper_value();
    let _feature = feature_marker();
}
"#,
    );

    let output = run_kobo(&[s("build")], &project.root);

    assert_success(
        &output,
        "kobo build must preserve Cargo path dependencies and feature flags without Kobo metadata",
    );
    assert_not_contains(
        &project.read("target/kobo-gen/Cargo.toml"),
        "kobo-types-",
        "base Cargo compatibility must not require optional Kobo metadata packages",
    );
}

#[test]
fn workspace_dependency_inheritance_builds_without_rewriting_cargo_graph() {
    let project = TestProject::new("v11-cargo-workspace-inherited-dep");
    write_path_dependency(&project, "serde", Some("derive"));
    project.write(
        "Cargo.toml",
        r#"[package]
name = "workspace-inherited-dep"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true, features = ["derive"] }

[workspace]

[workspace.dependencies]
serde = { path = "serde" }
"#,
    );
    project.write("src/main.rs", "fn main() {}\n");
    project.main_file(
        r#"
fn main() {
    let _value = 1;
}
"#,
    );

    let output = run_kobo(&[s("build")], &project.root);

    assert_success(
        &output,
        "workspace-inherited Cargo dependencies should build through Kobo without metadata packages",
    );
    let generated = project.read("target/kobo-gen/Cargo.toml");
    assert_contains(
        &generated,
        "[workspace.dependencies]",
        "generated manifest should preserve workspace dependency metadata",
    );
    assert_contains(
        &generated,
        r#"serde = { path = "../../serde" }"#,
        "workspace dependency source path should be rewritten relative to the generated manifest",
    );
    assert_contains(
        &generated,
        r#"serde = { features = ["derive"], workspace = true }"#,
        "generated dependency should preserve workspace inheritance and member feature requests",
    );
    assert!(
        !generated.contains("kobo-types-"),
        "workspace dependency inheritance should not force Kobo metadata packages:\n{generated}"
    );
}

#[test]
fn inspect_cargo_preserves_feature_mutation_from_cargo_manifest() {
    let project = TestProject::new("v11-cargo-feature-mutation");
    write_path_dependency(&project, "helper_dep", Some("fancy"));
    write_cargo_manifest(&project, "helper_dep", "helper_dep");
    let file = project.main_file("fn main() {}\n");

    let first_dir = project.root.join("target").join("first-cargo");
    let first = run_kobo(
        &[
            s("inspect"),
            path_arg(&file),
            s("--cargo"),
            path_arg(&first_dir),
        ],
        &project.root,
    );
    assert_success(&first, "initial inspect --cargo should succeed");
    let first_manifest = fs::read_to_string(first_dir.join("Cargo.toml"))
        .expect("first generated Cargo.toml should be readable");
    assert_contains(
        &first_manifest,
        r#"features = ["fancy"]"#,
        "generated Cargo.toml must preserve the original feature set",
    );

    let cargo = project
        .read("Cargo.toml")
        .replace(r#"features = ["fancy"]"#, r#"default-features = false"#);
    project.write("Cargo.toml", &cargo);

    let second_dir = project.root.join("target").join("second-cargo");
    let second = run_kobo(
        &[
            s("inspect"),
            path_arg(&file),
            s("--cargo"),
            path_arg(&second_dir),
        ],
        &project.root,
    );
    assert_success(&second, "mutated inspect --cargo should succeed");
    let second_manifest = fs::read_to_string(second_dir.join("Cargo.toml"))
        .expect("second generated Cargo.toml should be readable");
    assert_contains(
        &second_manifest,
        "default-features = false",
        "generated Cargo.toml must reflect the mutated Cargo feature shape",
    );
    assert_ne!(
        first_manifest, second_manifest,
        "Cargo metadata evidence must change when the source Cargo feature request changes"
    );
}

#[test]
fn build_script_errors_are_passthrough_not_generic_kobo_errors() {
    let project = TestProject::new("v11-cargo-build-rs-passthrough");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-build-script-failure"
version = "0.1.0"
edition = "2021"
build = "build.rs"
"#,
    );
    project.write(
        "build.rs",
        r#"fn main() {
    panic!("v11 build script sentinel");
}
"#,
    );
    project.main_file("fn main() {}\n");

    let output = run_kobo(&[s("build")], &project.root);

    assert_failure(&output, "failing build script must make kobo build fail");
    assert_contains(
        &output.combined(),
        "v11 build script sentinel",
        "Cargo/rustc context from build.rs must be visible to the user",
    );
    assert_contains(
        &output.combined(),
        "K0128",
        "Cargo compatibility regressions should use the v0.11 diagnostic code",
    );
}

#[test]
fn doctor_reports_distinct_dependency_identity_beyond_crate_name() {
    let project = TestProject::new("v11-cargo-identity");
    write_path_dependency(&project, "helper_dep", Some("fancy"));
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-cargo-identity"
version = "0.1.0"
edition = "2021"

[dependencies]
local_helper = { package = "helper_dep", path = "helper_dep", features = ["fancy"] }
helper_dep = { version = "0.1.0", path = "helper_dep", default-features = false }
"#,
    );

    let output = run_kobo(&[s("doctor"), s("--deps"), s("--json")], &project.root);
    assert_success(&output, "doctor --deps should inspect Cargo.toml");
    let report: Value =
        serde_json::from_str(&output.stdout).expect("doctor --deps output should be JSON");
    let dependencies = report["dependencies"]
        .as_array()
        .expect("dependencies should be an array");
    let identities = dependencies
        .iter()
        .filter_map(|dependency| dependency["identity"].as_str())
        .collect::<Vec<_>>();

    assert!(
        identities
            .iter()
            .any(|identity| identity.contains("local_helper")
                && identity.contains("package=helper_dep")
                && identity.contains("features=fancy")),
        "renamed path dependency identity must include alias, package, source, and features: {report}"
    );
    assert!(
        identities.iter().any(|identity| identity.contains("helper_dep")
            && identity.contains("path=helper_dep")
            && identity.contains("default-features=false")),
        "same package name from another dependency entry must not collapse by crate name alone: {report}"
    );
}

#[test]
fn doctor_uses_resolved_cargo_metadata_for_common_dependency_graph() {
    let project = TestProject::new("v11-cargo-common-metadata");
    write_path_dependency(&project, "helper_dep", Some("fancy"));
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-cargo-common-metadata"
version = "0.1.0"
edition = "2021"

[dependencies]
serde_json = "1"
anyhow = "1"
clap = { version = "4", features = ["derive"] }
tokio = { version = "1", features = ["rt", "macros", "sync", "time"] }
thiserror = "2"
local_helper = { package = "helper_dep", path = "helper_dep", features = ["fancy"] }

[target.'cfg(windows)'.dependencies]
win_helper = { package = "helper_dep", path = "helper_dep", default-features = false }
"#,
    );

    let output = run_kobo(&[s("doctor"), s("--deps"), s("--json")], &project.root);
    assert_success(
        &output,
        "doctor --deps should inspect common Cargo metadata without Kobo metadata",
    );
    let report: Value =
        serde_json::from_str(&output.stdout).expect("doctor --deps output should be JSON");
    assert_contains(
        &report.to_string(),
        "resolved_packages",
        "doctor should include cargo metadata package identities",
    );
    for expected in ["serde_json", "anyhow", "clap", "tokio", "thiserror"] {
        assert_contains(
            &report.to_string(),
            expected,
            "common dependency should be present in dependency evidence",
        );
    }
    assert_contains(
        &report.to_string(),
        "target.'cfg(windows)'.dependencies",
        "target-specific dependency section should remain visible",
    );
    assert_contains(
        &report.to_string(),
        "features=derive",
        "resolved or manifest identity should include feature sets",
    );
    assert_contains(
        &report.to_string(),
        "path=helper_dep",
        "path dependency identity should remain distinct from registry packages",
    );
}
