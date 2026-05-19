mod v09_common;

use std::path::Path;
use std::time::Duration;

use v09_common::{
    assert_contains, assert_failure, assert_success, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);
const ACME_DECLARATION: &str = "schema_version = 0\n\n[crate]\nname = \"acme_http\"\nversion = \"0.4.1\"\nsource = \"registry-package\"\n\n[[type]]\npath = \"acme_http::LiveClient\"\nkind = \"struct\"\n\n[[function]]\npath = \"acme_http::live_get\"\nreturns = \"acme_http::LiveClient\"\nsimulation = \"typed\"\ndeterminism = \"deterministic\"\n";
const ACME_DECLARATION_HASH: &str = "8f6a0910ae72032c";
const ACME_TYPES_PACKAGE: &str = "schema_version = 1\npackage = \"kobo-types-acme-http\"\nversion = \"0.4.1\"\nkind = \"types\"\ncompatible_crate = \">=0.4.0,<0.5.0\"\nsigned_by = \"kobo-local\"\ndeclaration_path = \"../declarations/acme_http.kobo.d.toml\"\ndeclaration_hash = \"8f6a0910ae72032c\"\n";
const ACME_ADAPTER_PACKAGE: &str = "schema_version = 1\npackage = \"kobo-adapter-acme-http\"\nversion = \"0.4.1\"\nkind = \"adapter\"\ncompatible_crate = \">=0.4.0,<0.5.0\"\nsigned_by = \"kobo-local\"\nadapter_runtime = \"acme_http::kobo_adapter::Adapter\"\ncapture = \"boundary-io\"\n";
const ACME_TYPES_SHA256: &str = "8c8b79203c593b583ed3a86a16af116a60c221bbe277944ef72851b35396c9bf";
const ACME_ADAPTER_SHA256: &str =
    "43784daf84c26910bfb738e415276ff22e9b334f4311852f37479bc6958b7eb0";

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
fn kobo_add_promised_registry_crate_without_explicit_version() {
    let project = TestProject::new("v11-kobo-add-serde-json");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-kobo-add-serde-json"
version = "0.1.0"
edition = "2021"
"#,
    );

    let add = run_kobo(&[s("add"), s("serde_json")], &project.root);

    assert_success(
        &add,
        "kobo add serde_json should work through the built-in package registry",
    );
    let cargo = project.read("Cargo.toml");
    assert_contains(
        &cargo,
        r#"serde_json = { version = "1" }"#,
        "registry-backed kobo add should write a concrete Cargo version",
    );
    assert!(
        !cargo.contains(r#"version = "*""#),
        "wildcard versions are not production metadata: {cargo}"
    );
    assert!(
        !project.root.join("Kobo.toml").exists(),
        "plain kobo add should not force optional Kobo metadata packages"
    );
}

fn write_local_registry(project: &TestProject) {
    project.write(
        ".kobo/registry/packages/acme-types.toml",
        ACME_TYPES_PACKAGE,
    );
    project.write(
        ".kobo/registry/declarations/acme_http.kobo.d.toml",
        ACME_DECLARATION,
    );
    project.write(
        ".kobo/registry/packages/acme-adapter.toml",
        ACME_ADAPTER_PACKAGE,
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
    project.write(
        ".kobo/registry/index.toml",
        r#"schema_version = 1
name = "local-v0.11"
trust_policy = "workspace-pinned"
trusted_signers = ["kobo-local"]
min_kobo_version = "0.11"

[[crate]]
name = "acme_http"
cargo_version = "0.4.1"
signed_by = "kobo-local"
source_path = "vendor/acme_http"
public_types = ["StaleRegistryType"]
public_functions = ["stale_registry_fn"]
summary_path = ".kobo-summary/acme_http.json"
summary_hash = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[crate.types]
package = "kobo-types-acme-http"
version = "0.4.1"
metadata_path = "packages/acme-types.toml"
checksum = "sha256:8c8b79203c593b583ed3a86a16af116a60c221bbe277944ef72851b35396c9bf"
compatible_crate = ">=0.4.0,<0.5.0"

[crate.adapter]
package = "kobo-adapter-acme-http"
version = "0.4.1"
metadata_path = "packages/acme-adapter.toml"
checksum = "sha256:43784daf84c26910bfb738e415276ff22e9b334f4311852f37479bc6958b7eb0"
compatible_crate = ">=0.4.0,<0.5.0"
"#,
    );
}

#[test]
fn third_party_registry_index_drives_add_and_metadata_packages() {
    let project = TestProject::new("v11-third-party-registry");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-third-party-registry"
version = "0.1.0"
edition = "2021"
"#,
    );
    write_local_registry(&project);

    let add = run_kobo(&[s("add"), s("acme_http")], &project.root);
    assert_success(
        &add,
        "kobo add should discover package versions from a validated registry index",
    );
    assert_contains(
        &project.read("Cargo.toml"),
        r#"acme_http = { version = "0.4.1" }"#,
        "registry discovery should write the compatible Cargo version",
    );
    assert_contains(
        &add.stderr,
        "optional Kobo metadata available",
        "kobo add should suggest optional metadata packages without forcing them",
    );
    for expected in [
        "declaration metadata package: kobo-types-acme-http 0.4.1",
        "simulation adapter package: kobo-adapter-acme-http 0.4.1",
        "Add now? [types] [types+adapter] [skip]",
        "noninteractive: run `kobo add-types acme_http` or `kobo add-adapter acme_http`",
    ] {
        assert_contains(
            &add.stderr,
            expected,
            "kobo add should provide the full optional metadata suggestion UX",
        );
    }

    let add_types = run_kobo(&[s("add-types"), s("acme_http")], &project.root);
    assert_success(
        &add_types,
        "kobo add-types should resolve a third-party metadata package from the registry index",
    );
    let add_adapter = run_kobo(&[s("add-adapter"), s("acme_http")], &project.root);
    assert_success(
        &add_adapter,
        "kobo add-adapter should resolve a third-party adapter package from the registry index",
    );
    let kobo_toml = project.read("Kobo.toml");
    for expected in [
        r#"package = "kobo-types-acme-http""#,
        r#"package = "kobo-adapter-acme-http""#,
        r#"source = "registry-index""#,
        r#"registry = "local-v0.11""#,
        &format!(r#"checksum = "sha256:{ACME_TYPES_SHA256}""#),
        &format!(r#"checksum = "sha256:{ACME_ADAPTER_SHA256}""#),
        r#"compatible_crate = ">=0.4.0,<0.5.0""#,
        &format!(r#"declaration_hash = "{ACME_DECLARATION_HASH}""#),
        r#"adapter_runtime = "acme_http::kobo_adapter::Adapter""#,
        r#"capture = "boundary-io""#,
        r#"trust_policy = "workspace-pinned""#,
        r#"signed_by = "kobo-local""#,
        r#"validated = true"#,
        r#"[[ecosystem.summary]]"#,
        r#"crate = "acme_http""#,
        r#"path = ".kobo-summary/acme_http.json""#,
        r#"hash = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa""#,
    ] {
        assert_contains(
            &kobo_toml,
            expected,
            "registry-backed metadata should retain trust, compatibility, checksum, and summary propagation evidence",
        );
    }
}

#[test]
fn untrusted_or_incomplete_registry_metadata_is_rejected() {
    let project = TestProject::new("v11-untrusted-registry");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-untrusted-registry"
version = "0.1.0"
edition = "2021"
"#,
    );
    project.write(
        ".kobo/registry/index.toml",
        r#"schema_version = 1
name = "untrusted-v0.11"
trusted = false

[[crate]]
name = "mystery"
cargo_version = "1"

[crate.types]
package = "kobo-types-mystery"
version = "1"
"#,
    );

    let output = run_kobo(&[s("add-types"), s("mystery")], &project.root);
    assert_failure(
        &output,
        "untrusted registry metadata must be rejected instead of recorded",
    );
    assert_contains(
        &output.combined(),
        "K0128",
        "registry validation failures should use the ecosystem package diagnostic",
    );
    assert!(
        !project.root.join("Kobo.toml").exists(),
        "failed registry validation must not write trusted Kobo metadata"
    );
}

#[test]
fn registry_metadata_package_checksum_mismatch_is_rejected() {
    let project = TestProject::new("v11-registry-checksum-mismatch");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-registry-checksum-mismatch"
version = "0.1.0"
edition = "2021"
"#,
    );
    write_local_registry(&project);
    project.write(
        ".kobo/registry/packages/acme-types.toml",
        "schema_version = 1\npackage = \"tampered\"\n",
    );

    let output = run_kobo(&[s("add-types"), s("acme_http")], &project.root);

    assert_failure(
        &output,
        "tampered metadata package content must fail checksum validation",
    );
    assert_contains(
        &output.combined(),
        "checksum mismatch",
        "registry validation should compare actual package file checksums",
    );
}

#[test]
fn registry_types_package_must_pin_usable_declaration_before_validation() {
    let project = TestProject::new("v11-registry-types-without-declaration");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-registry-types-without-declaration"
version = "0.1.0"
edition = "2021"
"#,
    );
    write_local_registry(&project);
    project.write(
        ".kobo/registry/packages/acme-types.toml",
        "schema_version = 1\npackage = \"kobo-types-acme-http\"\nversion = \"0.4.1\"\nkind = \"types\"\ncompatible_crate = \">=0.4.0,<0.5.0\"\nsigned_by = \"kobo-local\"\n",
    );
    let index = project.read(".kobo/registry/index.toml").replace(
        ACME_TYPES_SHA256,
        "eaed1ce30beab6c4a4d28de4f4a4fb34e7634aade3fde528b8d462d27e55e897",
    );
    project.write(".kobo/registry/index.toml", &index);

    let output = run_kobo(&[s("add-types"), s("acme_http")], &project.root);

    assert_failure(
        &output,
        "registry types metadata must be rejected before validated=true when no declaration is pinned",
    );
    assert_contains(
        &output.combined(),
        "declaration_path",
        "types package validation should require a usable declaration file",
    );
    assert!(
        !project.root.join("Kobo.toml").exists(),
        "failed types package validation must not write validated metadata"
    );
}

#[test]
fn registry_metadata_compatibility_uses_consuming_dependency_version() {
    let project = TestProject::new("v11-registry-consuming-version");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-registry-consuming-version"
version = "0.1.0"
edition = "2021"

[dependencies]
acme_http = "0.3.9"
"#,
    );
    write_local_registry(&project);

    let output = run_kobo(&[s("add-types"), s("acme_http")], &project.root);

    assert_failure(
        &output,
        "metadata packages must be rejected when the consuming dependency version is incompatible",
    );
    assert_contains(
        &output.combined(),
        "incompatible consuming dependency version",
        "compatibility checks should use the project's Cargo dependency version",
    );
}

#[test]
fn registry_source_path_must_stay_inside_workspace_and_match_crate_identity() {
    let project = TestProject::new("v11-registry-source-identity");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-registry-source-identity"
version = "0.1.0"
edition = "2021"
"#,
    );
    project.write(
        ".kobo/registry/index.toml",
        r#"schema_version = 1
name = "local-v0.11"
trust_policy = "workspace-pinned"
trusted_signers = ["kobo-local"]

[[crate]]
name = "acme_http"
cargo_version = "0.4.1"
signed_by = "kobo-local"
source_path = "../outside"
"#,
    );

    let escaped = run_kobo(&[s("bindgen"), s("acme_http")], &project.root);
    assert_failure(
        &escaped,
        "registry source_path must be confined to the workspace",
    );
    assert_contains(
        &escaped.combined(),
        "source_path escapes workspace",
        "escaped source paths should be rejected before bindgen",
    );

    project.write(
        ".kobo/registry/index.toml",
        r#"schema_version = 1
name = "local-v0.11"
trust_policy = "workspace-pinned"
trusted_signers = ["kobo-local"]

[[crate]]
name = "acme_http"
cargo_version = "0.4.1"
signed_by = "kobo-local"
source_path = "vendor/wrong"
"#,
    );
    project.write(
        "vendor/wrong/Cargo.toml",
        r#"[package]
name = "wrong"
version = "0.4.1"
edition = "2021"
"#,
    );
    project.write("vendor/wrong/src/lib.rs", "pub struct Wrong;\n");

    let mismatch = run_kobo(&[s("bindgen"), s("acme_http")], &project.root);
    assert_failure(
        &mismatch,
        "registry source_path must match the indexed crate identity",
    );
    assert_contains(
        &mismatch.combined(),
        "source_path crate identity mismatch",
        "bindgen should validate registry source crate name/version",
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
    assert_contains(
        &kobo_toml,
        r#"source = "registry""#,
        "metadata packages should come from the registry layer, not only a name convention",
    );
    assert_contains(
        &kobo_toml,
        r#"registry = "builtin-v0.11""#,
        "metadata packages should record the registry that supplied the annotation",
    );
    assert_contains(
        &kobo_toml,
        r#"version = "0.8""#,
        "types package metadata should preserve registry package version",
    );
    assert_contains(
        &kobo_toml,
        r#"version = "1""#,
        "adapter package metadata should preserve registry package version",
    );
    for expected in [
        "metadata_path = \"builtin://builtin-v0.11/packages/kobo-types-sqlx-0.8.toml\"",
        "declaration_path = \"builtin://builtin-v0.11/declarations/sqlx-0.8.kobo.d.toml\"",
        "metadata_path = \"builtin://builtin-v0.11/packages/kobo-adapter-tokio-1.toml\"",
        "checksum = \"sha256:",
        "declaration_hash = \"",
        "trust_policy = \"builtin-reviewed\"",
        "signed_by = \"kobo-core\"",
        "adapter_runtime = \"kobo_builtin::kobo_adapter_tokio::Adapter\"",
        "capture = \"facade-call-capture\"",
        "validated = true",
    ] {
        assert_contains(
            &kobo_toml,
            expected,
            "built-in registry packages should carry first-party package validation evidence",
        );
    }
}

#[test]
fn builtin_thiserror_types_package_carries_declaration_hash() {
    let project = TestProject::new("v11-add-types-thiserror");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-add-types-thiserror"
version = "0.1.0"
edition = "2021"
"#,
    );

    let add_types = run_kobo(&[s("add-types"), s("thiserror")], &project.root);
    assert_success(
        &add_types,
        "kobo add-types thiserror should record validated built-in declaration metadata",
    );

    let kobo_toml = project.read("Kobo.toml");
    for expected in [
        r#"package = "kobo-types-thiserror""#,
        r#"version = "2""#,
        r#"declaration_path = "builtin://builtin-v0.11/declarations/thiserror-2.kobo.d.toml""#,
        r#"declaration_hash = ""#,
        r#"validated = true"#,
    ] {
        assert_contains(
            &kobo_toml,
            expected,
            "built-in thiserror package should carry complete declaration evidence",
        );
    }
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
        &project.read("Kobo.toml"),
        r#"name = "serde_json""#,
        "init --from-cargo should seed dependency metadata from Cargo.toml",
    );
    assert_contains(
        &project.read("Kobo.toml"),
        r#"section = "dependencies""#,
        "init --from-cargo should preserve the imported Cargo dependency section",
    );
    assert_contains(
        &project.read("src/main.rs"),
        "fn main() {}",
        "init --from-cargo must not rewrite existing Rust source",
    );
}

#[test]
fn add_supports_dev_build_and_target_dependency_sections() {
    let project = TestProject::new("v11-add-section-targeting");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "v11-add-section-targeting"
version = "0.1.0"
edition = "2021"
"#,
    );

    let dev = run_kobo(&[s("add"), s("insta@1"), s("--dev")], &project.root);
    assert_success(&dev, "kobo add --dev should edit dev-dependencies");
    let build = run_kobo(&[s("add"), s("cc@1"), s("--build")], &project.root);
    assert_success(&build, "kobo add --build should edit build-dependencies");
    let target = run_kobo(
        &[
            s("add"),
            s("windows-sys@0.52"),
            s("--target"),
            s("cfg(windows)"),
        ],
        &project.root,
    );
    assert_success(
        &target,
        "kobo add --target should edit target-specific dependencies",
    );

    let cargo = project.read("Cargo.toml");
    for expected in [
        "[dev-dependencies]",
        "insta",
        "[build-dependencies]",
        "cc",
        "[target.\"cfg(windows)\".dependencies]",
        "windows-sys",
    ] {
        assert_contains(
            &cargo,
            expected,
            "kobo add should support Cargo dependency section breadth",
        );
    }
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

[dev-dependencies]
insta = "1"

[build-dependencies]
cc = { version = "1", default-features = false }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.52", package = "windows-sys", features = ["Win32_Foundation"] }
"#,
    );
    project.write("src/main.rs", "fn main() {}\n");

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
    let kobo_toml = project.read("Kobo.toml");
    for expected in [
        r#"name = "insta""#,
        r#"section = "dev-dependencies""#,
        r#"name = "cc""#,
        r#"section = "build-dependencies""#,
        r#"default_features = false"#,
        r#"name = "windows-sys""#,
        r#"section = "target.dependencies""#,
        r#"target = "cfg(windows)""#,
        r#"features = ["Win32_Foundation"]"#,
        r#"source = "registry""#,
        r#"migration_source = "cargo-metadata""#,
        r#"managed_by = "kobo migrate-cargo-deps""#,
    ] {
        assert_contains(
            &kobo_toml,
            expected,
            "migrate-cargo-deps should preserve real Cargo dependency sections and metadata",
        );
    }

    let second = run_kobo(&[s("migrate-cargo-deps")], &project.root);
    assert_success(
        &second,
        "migrate-cargo-deps should be safe to rerun after Cargo.toml changes",
    );
    let rerun_kobo_toml = project.read("Kobo.toml");
    assert_eq!(
        rerun_kobo_toml.matches("[[ecosystem.crate]]").count(),
        4,
        "migration should upsert managed dependency metadata instead of appending duplicates:\n{rerun_kobo_toml}"
    );
}

#[test]
fn migrate_cargo_deps_imports_workspace_member_dependencies() {
    let project = TestProject::new("v11-migrate-workspace-deps");
    project.write(
        "Cargo.toml",
        r#"[workspace]
members = ["app", "worker"]
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
    project.write(
        "worker/Cargo.toml",
        r#"[package]
name = "worker"
version = "0.1.0"
edition = "2021"

[build-dependencies]
cc = "1"
"#,
    );

    let output = run_kobo(&[s("migrate-cargo-deps")], &project.root);
    assert_success(
        &output,
        "migrate-cargo-deps should traverse workspace member manifests",
    );
    let kobo_toml = project.read("Kobo.toml");
    for expected in [
        r#"member = "app""#,
        r#"name = "serde""#,
        r#"member = "worker""#,
        r#"name = "cc""#,
        r#"section = "build-dependencies""#,
    ] {
        assert_contains(
            &kobo_toml,
            expected,
            "workspace member dependencies should be imported as optional ecosystem debt",
        );
    }
}

#[test]
fn migrate_cargo_deps_expands_workspace_member_globs() {
    let project = TestProject::new("v11-migrate-workspace-globs");
    project.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]
"#,
    );
    project.write(
        "crates/api/Cargo.toml",
        r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.7"
"#,
    );
    project.write(
        "crates/job/Cargo.toml",
        r#"[package]
name = "job"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
insta = "1"
"#,
    );

    let output = run_kobo(&[s("migrate-cargo-deps")], &project.root);
    assert_success(
        &output,
        "migrate-cargo-deps should expand Cargo workspace member globs",
    );
    let kobo_toml = project.read("Kobo.toml");
    for expected in [
        r#"member = "crates/api""#,
        r#"name = "axum""#,
        r#"member = "crates/job""#,
        r#"name = "insta""#,
        r#"section = "dev-dependencies""#,
    ] {
        assert_contains(
            &kobo_toml,
            expected,
            "globbed workspace members should be imported",
        );
    }
}
