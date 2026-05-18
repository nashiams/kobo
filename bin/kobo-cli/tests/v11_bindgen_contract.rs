mod v09_common;

use std::path::Path;
use std::time::Duration;

use v09_common::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

fn write_fixture_crate(project: &TestProject, name: &str, body: &str) -> std::path::PathBuf {
    let crate_dir = project.root.join(name);
    project.write(
        &format!("{name}/Cargo.toml"),
        &format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
"#
        ),
    );
    project.write(&format!("{name}/src/lib.rs"), body);
    crate_dir
}

#[test]
fn bindgen_positional_crate_uses_builtin_registry_metadata() {
    let project = TestProject::new("v11-bindgen-registry-sqlx");

    let output = run_kobo(&[s("bindgen"), s("sqlx")], &project.root);

    assert_success(
        &output,
        "kobo bindgen sqlx should generate a registry-backed declaration draft",
    );
    assert_contains(
        &output.stdout,
        r#"name = "sqlx""#,
        "registry bindgen should target the requested crate",
    );
    assert_contains(
        &output.stdout,
        r#"package_source = "registry""#,
        "registry bindgen should disclose that metadata came from the built-in registry",
    );
    assert_contains(
        &output.stdout,
        r#"registry = "builtin-v0.11""#,
        "registry bindgen should identify the metadata registry layer",
    );
    assert_contains(
        &output.stdout,
        "sqlx::Pool",
        "registry bindgen should emit useful public metadata, not just a crate-name template",
    );
    assert_contains(
        &output.combined(),
        "K0127",
        "registry bindgen drafts should still require review",
    );
}

#[test]
fn bindgen_crate_flag_preserves_feature_request_in_registry_draft() {
    let project = TestProject::new("v11-bindgen-registry-reqwest");

    let output = run_kobo(
        &[
            s("bindgen"),
            s("--crate"),
            s("reqwest"),
            s("--features"),
            s("json,rustls-tls"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "kobo bindgen --crate reqwest --features ... should generate a draft",
    );
    assert_contains(
        &output.stdout,
        r#"name = "reqwest""#,
        "bindgen --crate should target the named crate",
    );
    assert_contains(
        &output.stdout,
        r#"features = ["json", "rustls-tls"]"#,
        "bindgen should preserve requested registry feature metadata",
    );
    assert_contains(
        &output.stdout,
        "reqwest::Client",
        "registry draft should include known public API metadata",
    );
    assert_contains(
        &output.stdout,
        "review_question",
        "registry bindgen should remain a review-required draft",
    );
}

#[test]
fn bindgen_path_outputs_schema_v0_with_review_questions() {
    let project = TestProject::new("v11-bindgen-path");
    let crate_dir = write_fixture_crate(
        &project,
        "payments",
        r#"
pub struct Transaction {}

impl Transaction {
    pub fn commit(self) {}
    pub fn rollback(self) {}
}

pub fn begin_transaction() -> Transaction {
    Transaction {}
}

pub fn charge_card() {
    std::env::var("PAYMENTS_GATEWAY").ok();
}
"#,
    );

    let output = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );

    assert_success(
        &output,
        "bindgen --path should generate a declaration draft",
    );
    assert_contains(
        &output.stdout,
        "schema_version = 0",
        "bindgen output must declare schema v0",
    );
    assert_contains(
        &output.stdout,
        r#"source = "bindgen""#,
        "bindgen output must be marked as a draft source",
    );
    assert_contains(
        &output.stdout,
        "review_required = true",
        "bindgen drafts should be explicitly review-required",
    );
    assert_contains(
        &output.stdout,
        "source_hash = ",
        "bindgen should label package source/API hashes as source_hash",
    );
    assert_contains(
        &output.stdout,
        "declaration_hash = ",
        "bindgen should include a self-checkable declaration hash",
    );
    assert!(
        !output.stdout.contains("summary_hash = "),
        "bindgen should not use .kobo-summary terminology for source/API hashes:\n{}",
        output.stdout
    );
    assert_contains(
        &output.combined(),
        "K0127",
        "bindgen should emit the v0.11 review-required diagnostic",
    );
    assert_contains(
        &output.stdout,
        "payments::Transaction",
        "resource-like public type should appear in the draft declaration",
    );
    assert_contains(
        &output.stdout,
        "review_question",
        "ambiguous effects must become review questions, not trusted facts",
    );
}

#[test]
fn bindgen_follows_public_modules_reexports_and_type_shapes() {
    let project = TestProject::new("v11-bindgen-modules");
    let crate_dir = write_fixture_crate(
        &project,
        "gateway",
        r#"
pub mod api;
pub use api::Client as GatewayClient;
"#,
    );
    project.write(
        "gateway/src/api.rs",
        r#"
pub enum Event { Started }
pub trait SendGateway { fn send(&self); fn flush(&mut self); }
pub type RequestId<T> = std::result::Result<T, String>;

pub struct Client<T> { marker: std::marker::PhantomData<T> }

impl<T> Client<T> {
    pub fn close(self) {}
}

pub fn send_request() -> Client<String> { Client { marker: std::marker::PhantomData } }
"#,
    );

    let output = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );

    assert_success(&output, "bindgen should inspect module-based public API");
    for expected in [
        "gateway::api::Event",
        "gateway::api::SendGateway",
        "gateway::api::RequestId",
        "gateway::api::Client",
        "gateway::api::send_request",
        "gateway::GatewayClient",
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "module/reexport public API should appear in declaration draft",
        );
    }
    for expected in [
        r#"kind = "enum""#,
        r#"variants = ["Started"]"#,
        r#"kind = "trait""#,
        r#"trait_methods = ["send", "flush"]"#,
        r#"kind = "type_alias""#,
        r#"generics = ["T"]"#,
        r#"alias_target = "std :: result :: Result < T , String >""#,
        r#"target = "gateway::api::Client""#,
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "bindgen should include richer public API facts for review",
        );
    }
}

#[test]
fn public_api_mutation_changes_bindgen_output() {
    let project = TestProject::new("v11-bindgen-mutation");
    let crate_dir = write_fixture_crate(
        &project,
        "queue",
        r#"
pub struct Delivery {}
pub fn receive() -> Delivery { Delivery {} }
"#,
    );
    let first = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );
    assert_success(&first, "first bindgen run should succeed");

    project.write(
        "queue/src/lib.rs",
        r#"
pub struct Delivery {}
pub struct AckToken {}
pub fn receive() -> Delivery { Delivery {} }
pub fn ack_token() -> AckToken { AckToken {} }
"#,
    );
    let second = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );
    assert_success(&second, "second bindgen run should succeed");

    assert_ne!(
        first.stdout, second.stdout,
        "bindgen draft must depend on the crate public API, not a canned crate name template"
    );
    assert_contains(
        &second.stdout,
        "queue::AckToken",
        "mutated public API should be reflected in the draft declaration",
    );
}
