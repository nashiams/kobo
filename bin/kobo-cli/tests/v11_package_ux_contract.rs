mod v09_common;

use std::path::Path;
use std::time::Duration;

use v09_common::{
    assert_contains, assert_success, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

#[test]
fn kobo_add_edits_cargo_metadata_first_and_keeps_metadata_optional() {
    let project = TestProject::new("v11-kobo-add");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-kobo-add"
version = "0.1.0"
edition = "2021"
"#,
    );

    let add = run_kobo(
        &[
            s("add"),
            s("tokio@1.36"),
            s("--features"),
            s("rt,macros,sync,time"),
            s("--no-default-features"),
        ],
        &project.root,
    );
    assert_success(&add, "kobo add should edit Cargo.toml");
    let cargo = project.read("Cargo.toml");
    assert_contains(
        &cargo,
        r#"tokio = { version = "1.36", default-features = false, features = ["rt", "macros", "sync", "time"] }"#,
        "kobo add must preserve requested Cargo feature metadata",
    );
    assert!(
        !cargo.contains(r#"version = "*""#),
        "kobo add must avoid wildcard dependency versions: {cargo}"
    );
    assert!(
        !cargo.contains("kobo-types-") && !cargo.contains("kobo-adapter-"),
        "kobo add must not make Kobo metadata packages mandatory: {cargo}"
    );
}

#[test]
fn kobo_add_supports_workspace_dependency_section_and_path_source() {
    let project = TestProject::new("v11-kobo-add-workspace");
    project.write(
        "Cargo.toml",
        r#"[workspace]
members = ["app"]
"#,
    );

    let output = run_kobo(
        &[s("add"), s("local_dep"), s("--path"), s("../local_dep")],
        &project.root,
    );

    assert_success(
        &output,
        "kobo add at a workspace root should edit workspace dependency metadata",
    );
    let cargo = project.read("Cargo.toml");
    assert_contains(
        &cargo,
        "[workspace.dependencies]",
        "workspace roots should use workspace dependency metadata",
    );
    assert_contains(
        &cargo,
        r#"local_dep = { path = "../local_dep" }"#,
        "path dependencies should preserve source identity instead of forcing registry versions",
    );
}

#[test]
fn kobo_add_can_target_workspace_member_manifest() {
    let project = TestProject::new("v11-kobo-add-member");
    project.write(
        "Cargo.toml",
        r#"[workspace]
members = ["app"]
"#,
    );
    project.write(
        "app/Cargo.toml",
        r#"[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
"#,
    );

    let output = run_kobo(
        &[
            s("add"),
            s("tokio@1.36"),
            s("--member"),
            s("app"),
            s("--features"),
            s("rt,macros"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "kobo add --member should edit the selected workspace member manifest",
    );
    let root_cargo = project.read("Cargo.toml");
    let app_cargo = project.read("app/Cargo.toml");
    assert!(
        !root_cargo.contains("tokio"),
        "workspace root should not receive member-scoped dependency edits:\n{root_cargo}"
    );
    assert_contains(
        &app_cargo,
        r#"tokio = { version = "1.36", features = ["rt", "macros"] }"#,
        "selected member manifest should receive the requested Cargo dependency",
    );
    assert_contains(
        &app_cargo,
        r#"serde = "1""#,
        "structured manifest editing must preserve existing dependencies",
    );
}

#[test]
fn kobo_add_types_and_adapter_record_optional_kobo_metadata() {
    let project = TestProject::new("v11-add-types-adapter");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-add-types-adapter"
version = "0.1.0"
edition = "2021"
"#,
    );

    let add_types = run_kobo(&[s("add-types"), s("sqlx")], &project.root);
    assert_success(
        &add_types,
        "kobo add-types should record declaration metadata",
    );
    let add_adapter = run_kobo(&[s("add-adapter"), s("tokio")], &project.root);
    assert_success(
        &add_adapter,
        "kobo add-adapter should record adapter metadata",
    );

    let kobo_toml = project.read("Kobo.toml");
    assert_contains(
        &kobo_toml,
        r#"package = "kobo-types-sqlx""#,
        "types package should be recorded in Kobo metadata",
    );
    assert_contains(
        &kobo_toml,
        r#"package = "kobo-adapter-tokio""#,
        "adapter package should be recorded in Kobo metadata",
    );
}

#[test]
fn init_from_cargo_merges_existing_project_without_source_rewrite() {
    let project = TestProject::new("v11-init-from-cargo");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "existing-rust-project"
version = "0.1.0"
edition = "2021"

[dependencies]
serde_json = "1"
"#,
    );
    project.write("src/main.rs", "fn main() {}\n");

    let output = run_kobo(&[s("init"), s("--from-cargo")], &project.root);
    assert_success(
        &output,
        "kobo init --from-cargo should create Kobo.toml for an existing Cargo project",
    );
    assert_contains(
        &project.read("Kobo.toml"),
        r#"[ecosystem]"#,
        "imported project should get ecosystem defaults",
    );
    assert_contains(
        &project.read("src/main.rs"),
        "fn main() {}",
        "init --from-cargo must not rewrite existing Rust source",
    );
}

#[test]
fn migrate_cargo_deps_imports_dependency_shape_as_optional_ecosystem_debt() {
    let project = TestProject::new("v11-migrate-cargo-deps");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "migrate-cargo-deps"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = { version = "0.12", features = ["json"] }
"#,
    );

    let output = run_kobo(&[s("migrate-cargo-deps")], &project.root);
    assert_success(
        &output,
        "kobo migrate-cargo-deps should seed optional ecosystem metadata",
    );
    assert_contains(
        &project.read("Kobo.toml"),
        r#"name = "reqwest""#,
        "Cargo dependency should be reflected as an ecosystem boundary candidate",
    );
    assert_contains(
        &project.read("Kobo.toml"),
        r#"policy = "opaque""#,
        "migration must not force exact replay metadata for unknown crates",
    );
}
