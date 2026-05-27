mod cli_common;

use std::path::Path;
use std::time::Duration;

use cli_common::{
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

fn write_featured_fixture_crate(
    project: &TestProject,
    name: &str,
    body: &str,
) -> std::path::PathBuf {
    let crate_dir = project.root.join(name);
    project.write(
        &format!("{name}/Cargo.toml"),
        &format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[features]
advanced = []
"#
        ),
    );
    project.write(&format!("{name}/src/lib.rs"), body);
    crate_dir
}

fn assert_function_contains(output: &str, path: &str, expected: &str, message: &str) {
    let needle = format!("path = \"{path}\"");
    let Some(block) = output
        .split("[[function]]")
        .find(|block| block.contains(&needle))
    else {
        panic!("missing function declaration block for {path}");
    };
    assert_contains(block, expected, message);
}

#[test]
fn bindgen_positional_crate_uses_builtin_registry_metadata() {
    let project = TestProject::new("ecosystem-bindgen-registry-sqlx");

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
        r#"api_extraction_source = "registry-seed""#,
        "registry bindgen fallback should disclose seed metadata instead of claiming source-backed public API extraction",
    );
    assert_contains(
        &output.stdout,
        "run kobo bindgen --path <crate-source>",
        "registry seed drafts should tell users how to request source-backed public API extraction",
    );
    assert_contains(
        &output.stdout,
        r#"registry = "builtin-ecosystem""#,
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
    assert_contains(
        &output.combined(),
        "registry seed declaration draft",
        "registry seed fallback must not describe itself as source-backed extraction",
    );
}

#[test]
fn bindgen_crate_flag_preserves_feature_request_in_registry_draft() {
    let project = TestProject::new("ecosystem-bindgen-registry-reqwest");

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
        r#"api_extraction_status = "seed-only-use---path-for-source-backed-api""#,
        "registry bindgen should not overstate static seed facts as rustdoc-expanded extraction",
    );
    assert_contains(
        &output.stdout,
        "review_question",
        "registry bindgen should remain a review-required draft",
    );
}

#[test]
fn bindgen_registry_crate_prefers_validated_source_path_over_static_public_list() {
    let project = TestProject::new("ecosystem-bindgen-registry-source-path");
    project.write(
        ".kobo/registry/index.toml",
        r#"schema_version = 1
name = "local-ecosystem"
trust_policy = "workspace-pinned"
trusted_signers = ["kobo-local"]

[[crate]]
name = "acme_http"
cargo_version = "0.4.1"
signed_by = "kobo-local"
source_path = "vendor/acme_http"
public_types = ["StaleRegistryType"]
public_functions = ["stale_registry_fn"]
"#,
    );
    project.write(
        "vendor/acme_http/Cargo.toml",
        r#"[package]
name = "acme_http"
version = "0.4.1"
edition = "2021"
"#,
    );
    project.write(
        "vendor/acme_http/src/lib.rs",
        r#"
pub struct LiveClient {}
pub fn live_get() -> LiveClient { LiveClient {} }
"#,
    );

    let output = run_kobo(&[s("bindgen"), s("acme_http")], &project.root);

    assert_success(
        &output,
        "registry bindgen should extract public API from a validated registry source path",
    );
    assert_contains(
        &output.stdout,
        "acme_http::LiveClient",
        "registry source_path should drive real public API extraction",
    );
    assert_contains(
        &output.stdout,
        "acme_http::live_get",
        "registry source_path functions should be emitted",
    );
    assert!(
        !output.stdout.contains("StaleRegistryType")
            && !output.stdout.contains("stale_registry_fn"),
        "registry source extraction must not use stale static public lists:\n{}",
        output.stdout
    );
}

#[test]
fn bindgen_positional_crate_uses_cargo_dependency_source_when_available() {
    let project = TestProject::new("ecosystem-bindgen-cargo-dependency");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "ecosystem-bindgen-cargo-dependency"
version = "0.1.0"
edition = "2021"

[dependencies]
acme_http = { path = "vendor/acme_http" }
"#,
    );
    project.write("src/lib.rs", "pub fn root() {}\n");
    project.write(
        "vendor/acme_http/Cargo.toml",
        r#"[package]
name = "acme_http"
version = "0.4.1"
edition = "2021"
"#,
    );
    project.write(
        "vendor/acme_http/src/lib.rs",
        r#"
pub struct LiveClient {}
pub fn live_get() -> LiveClient { LiveClient {} }
"#,
    );

    let output = run_kobo(&[s("bindgen"), s("acme_http")], &project.root);

    assert_success(
        &output,
        "bindgen should resolve real Cargo dependency sources before static registry fallback",
    );
    for expected in [
        r#"metadata_source = "cargo-metadata""#,
        r#"api_extraction_source = "cargo-metadata+checked-syn-public-api""#,
        r#"api_validation = "cargo-check --lib""#,
        r#"name = "acme_http""#,
        "acme_http::LiveClient",
        "acme_http::live_get",
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "positional bindgen should extract the dependency public API from Cargo metadata",
        );
    }
}

#[test]
fn bindgen_path_outputs_schema_zero_with_review_questions() {
    let project = TestProject::new("ecosystem-bindgen-path");
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
        "bindgen output must declare the initial schema revision",
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
        r#"api_extraction_status = "cargo-check-validated-public-api-draft""#,
        "bindgen --path should disclose Cargo-validated source-backed public API extraction",
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
        "bindgen should emit the ecosystem review-required diagnostic",
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
fn bindgen_path_emits_public_inherent_methods_as_api_functions() {
    let project = TestProject::new("ecosystem-bindgen-methods");
    let crate_dir = write_fixture_crate(
        &project,
        "method_crate",
        r#"
pub struct Client;

impl Client {
    pub fn new() -> Self { Client }
    pub fn get(&self) -> Response { Response }
    fn private_helper(&self) {}
}

pub struct Response;
"#,
    );

    let output = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );

    assert_success(
        &output,
        "bindgen --path should produce a declaration draft for public methods",
    );
    for expected in [
        r#"path = "method_crate::Client::new""#,
        r#"path = "method_crate::Client::get""#,
        r#"method_receiver = "method_crate::Client""#,
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "public inherent methods should be emitted as declaration functions",
        );
    }
    assert!(
        !output.stdout.contains("private_helper"),
        "private impl methods must not leak into bindgen public API output:\n{}",
        output.stdout
    );
}

#[test]
fn bindgen_path_uses_cargo_metadata_and_feature_cfg_public_api() {
    let project = TestProject::new("ecosystem-bindgen-cargo-metadata");
    let crate_dir = write_featured_fixture_crate(
        &project,
        "feature_api",
        r#"
mod internal;
pub use internal::Exported;

pub struct Always {}

#[cfg(feature = "advanced")]
pub struct Advanced {}

#[cfg(not(feature = "advanced"))]
pub struct Basic {}
"#,
    );
    project.write(
        "feature_api/src/internal.rs",
        r#"
pub struct Exported {}
"#,
    );

    let output = run_kobo(
        &[
            s("bindgen"),
            s("--path"),
            path_arg(&crate_dir),
            s("--features"),
            s("advanced"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "bindgen --path should use Cargo package metadata and enabled features",
    );
    for expected in [
        r#"metadata_source = "cargo-metadata""#,
        r#"api_extraction_source = "cargo-metadata+checked-syn-public-api""#,
        r#"api_validation = "cargo-check --lib""#,
        r#"metadata_status = "resolved""#,
        r#"manifest_path = "#,
        r#"target_kind = "lib""#,
        r#"features = ["advanced"]"#,
        "feature_api::Always",
        "feature_api::Advanced",
        "feature_api::Exported",
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "path bindgen should disclose cargo metadata and cfg-filtered public API",
        );
    }
    assert!(
        !output.stdout.contains("feature_api::Basic"),
        "disabled cfg(not(feature = \"advanced\")) API must not be emitted:\n{}",
        output.stdout
    );
}

#[test]
fn bindgen_path_uses_cargo_target_src_path_and_inline_modules() {
    let project = TestProject::new("ecosystem-bindgen-target-src-path");
    let crate_dir = project.root.join("custom_api");
    project.write(
        "custom_api/Cargo.toml",
        r#"[package]
name = "custom_api"
version = "0.1.0"
edition = "2021"

[lib]
path = "entry/main_api.rs"
"#,
    );
    project.write(
        "custom_api/entry/main_api.rs",
        r#"
pub mod inline_api {
    pub struct InlineClient {}

    pub fn connect() -> InlineClient {
        InlineClient {}
    }
}
"#,
    );

    let output = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );

    assert_success(
        &output,
        "bindgen should read Cargo target src_path instead of assuming src/lib.rs",
    );
    for expected in [
        "custom_api::inline_api::InlineClient",
        "custom_api::inline_api::connect",
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "bindgen should include public API from inline modules under the Cargo target path",
        );
    }
}

#[test]
fn bindgen_path_uses_conservative_cfg_for_unknown_predicates() {
    let project = TestProject::new("ecosystem-bindgen-unknown-cfg");
    let crate_dir = write_featured_fixture_crate(
        &project,
        "cfg_api",
        r#"
pub struct Always {}

#[cfg(kobo_private_build)]
pub struct UnknownCfgType {}
"#,
    );

    let output = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );

    assert_success(
        &output,
        "bindgen --path should not fail on unknown cfg predicates",
    );
    assert_contains(
        &output.stdout,
        r#"cfg_policy = "conservative""#,
        "bindgen should disclose conservative cfg evaluation",
    );
    assert_contains(
        &output.stdout,
        "cfg_api::Always",
        "ordinary public API should still be emitted",
    );
    assert!(
        output.stdout.contains("cfg_api::UnknownCfgType"),
        "unknown cfg-gated API should be preserved as review-required instead of silently dropped:\n{}",
        output.stdout
    );
    assert_contains(
        &output.stdout,
        "unresolved cfg predicate",
        "unresolved cfg-gated API should carry a review question",
    );
}

#[test]
fn bindgen_follows_public_modules_reexports_and_type_shapes() {
    let project = TestProject::new("ecosystem-bindgen-modules");
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
pub trait SendGateway {
    const MAX_IN_FLIGHT: usize;
    type Response;
    fn send(&self, request: RequestId<String>) -> Self::Response;
    fn flush(&mut self);
}
pub type RequestId<T> = std::result::Result<T, String>;

pub struct Client<T: Clone> { marker: std::marker::PhantomData<T> }

impl<T: Clone> Client<T> {
    pub fn close(self, reason: &str) {}
}

pub fn send_request(user_id: &str, limit: usize) -> Client<String> { Client { marker: std::marker::PhantomData } }
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
        r#"trait_associated_types = ["Response"]"#,
        r#"trait_associated_consts = ["MAX_IN_FLIGHT"]"#,
        r#"kind = "type_alias""#,
        r#"generics = ["T"]"#,
        r#"generic_bounds = ["T: Clone"]"#,
        r#"alias_target = "std :: result :: Result < T , String >""#,
        r#"parameters = ["user_id: & str", "limit: usize"]"#,
        r#"return_type_signature = "Client < String >""#,
        r#"determinism = "deterministic""#,
        r#"replay_policy = "typed-draft""#,
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
fn bindgen_path_emits_effect_and_replay_policy_fields() {
    let project = TestProject::new("ecosystem-bindgen-effects");
    let crate_dir = write_fixture_crate(
        &project,
        "effect_api",
        r#"
use std::env::var;

pub fn read_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn read_env_alias(name: &str) -> Option<String> {
    var(name).ok()
}

pub fn read_env_block_alias(name: &str) -> Option<String> {
    use std::env::var as read_var;
    read_var(name).ok()
}

pub fn pure_add(left: usize, right: usize) -> usize {
    left + right
}
"#,
    );

    let output = run_kobo(
        &[s("bindgen"), s("--path"), path_arg(&crate_dir)],
        &project.root,
    );

    assert_success(
        &output,
        "bindgen should emit function-level effects and replay policy metadata",
    );
    for expected in [
        r#"path = "effect_api::read_env""#,
        r#"parameters = ["name: & str"]"#,
        r#"return_type_signature = "Option < String >""#,
        r#"effects = ["environment"]"#,
        r#"determinism = "environment-dependent""#,
        r#"replay_policy = "review-required""#,
        r#"path = "effect_api::pure_add""#,
        r#"parameters = ["left: usize", "right: usize"]"#,
        r#"determinism = "deterministic""#,
        r#"replay_policy = "typed-draft""#,
    ] {
        assert_contains(
            &output.stdout,
            expected,
            "bindgen should emit production-review metadata for each public function",
        );
    }
    for path in [
        "effect_api::read_env_alias",
        "effect_api::read_env_block_alias",
    ] {
        assert_function_contains(
            &output.stdout,
            path,
            r#"effects = ["environment"]"#,
            "bindgen should resolve imported environment APIs inside each function body",
        );
        assert_function_contains(
            &output.stdout,
            path,
            r#"determinism = "environment-dependent""#,
            "import-resolved environment APIs should not be emitted as deterministic",
        );
        assert_function_contains(
            &output.stdout,
            path,
            r#"replay_policy = "review-required""#,
            "import-resolved environment APIs should require replay review",
        );
    }
}

#[test]
fn public_api_mutation_changes_bindgen_output() {
    let project = TestProject::new("ecosystem-bindgen-mutation");
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
