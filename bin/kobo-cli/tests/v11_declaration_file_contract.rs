mod v09_common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V11_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V11_TIMEOUT)
}

fn write_sqlx_project(project: &TestProject) {
    project.write(
        "Cargo.toml",
        r#"[package]
name = "decl_consumer"
version = "0.1.0"
edition = "2021"

[dependencies]
sqlx = "0.8"
"#,
    );
}

fn typed_sqlx_file(project: &TestProject) -> std::path::PathBuf {
    project.main_file(
        r#"
#[kobo::boundary(crate = "sqlx", policy = "typed", reason = "typed declaration supplied")]
use sqlx::Pool;

fn main() {
    let _pool = Pool::connect_lazy();
}
"#,
    )
}

fn valid_declaration(project: &TestProject, version: &str) {
    project.write(
        "sqlx.kobo.d.toml",
        &format!(
            r#"schema_version = 0

[crate]
name = "sqlx"
version = "{version}"
source = "bindgen"

[[type]]
path = "sqlx::Transaction"
must_call = ["commit", "rollback"]
resource = true

[[function]]
path = "sqlx::Pool::connect_lazy"
effects = []
simulation = "pure"
determinism = "deterministic"
"#
        ),
    );
}

#[test]
fn typed_policy_uses_declaration_version_and_hash() {
    let project = TestProject::new("v11-declaration-valid");
    write_sqlx_project(&project);
    valid_declaration(&project, "0.8");
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "valid typed declaration should satisfy replay-critical check",
    );
    assert_contains(
        &output.stdout,
        r#""declaration_version":"0.8""#,
        "typed boundary evidence should include declaration version",
    );
    assert_contains(
        &output.stdout,
        r#""declaration_hash""#,
        "typed boundary evidence should include declaration hash",
    );
    assert_contains(
        &output.stdout,
        "sqlx::Transaction",
        "declaration must surface resource obligations",
    );
    assert_contains(
        &output.stdout,
        "sqlx::Pool::connect_lazy",
        "declaration must surface covered function metadata",
    );
}

#[test]
fn documented_crate_declaration_path_is_discovered() {
    let project = TestProject::new("v11-declaration-crate-path");
    write_sqlx_project(&project);
    project.write(
        "crate.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"
source = "bindgen"

[[function]]
path = "sqlx::Pool::connect_lazy"
effects = []
simulation = "pure"
determinism = "deterministic"
"#,
    );
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "crate.kobo.d.toml should satisfy typed declaration lookup",
    );
    assert_contains(
        &output.stdout,
        "crate.kobo.d.toml",
        "documented declaration path should be reported in evidence",
    );
}

#[test]
fn typed_declaration_must_cover_boundary_call() {
    let project = TestProject::new("v11-declaration-coverage");
    write_sqlx_project(&project);
    project.write(
        "sqlx.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"
source = "bindgen"

[[function]]
path = "sqlx::Pool::other_call"
effects = ["database"]
simulation = "record"
"#,
    );
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(&output, "typed declaration must cover the boundary call");
    assert_contains(
        &output.combined(),
        "does not cover boundary call",
        "typed declarations should be validated against call coverage",
    );
}

#[test]
fn typed_declaration_rejects_replay_critical_effects_for_call() {
    let project = TestProject::new("v11-declaration-typed-effects");
    write_sqlx_project(&project);
    project.write(
        "sqlx.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"
source = "bindgen"

[[function]]
path = "sqlx::Pool::connect_lazy"
effects = ["database"]
simulation = "record"
"#,
    );
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "typed policy must not treat replay-critical effects as exact typed evidence",
    );
    assert_contains(
        &output.combined(),
        "still has replay-critical effects",
        "typed declarations should reject effectful call metadata for replay-critical paths",
    );
}

#[test]
fn typed_declaration_rejects_adapter_only_call_coverage() {
    let project = TestProject::new("v11-declaration-adapter-only-typed");
    write_sqlx_project(&project);
    project.write(
        "sqlx.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"
source = "bindgen"

[[adapter]]
path = "sqlx::Pool::connect_lazy"
package = "kobo-adapter-sqlx"
"#,
    );
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "adapter metadata alone must not satisfy typed exact boundary coverage",
    );
    assert_contains(
        &output.combined(),
        "does not cover boundary call",
        "typed coverage should require function-level declaration evidence",
    );
}

#[test]
fn declaration_with_wrong_declared_hash_emits_k0121() {
    let project = TestProject::new("v11-declaration-wrong-hash");
    write_sqlx_project(&project);
    project.write(
        "sqlx.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"
source = "bindgen"
summary_hash = "wrong-hash"

[[function]]
path = "sqlx::Pool::connect_lazy"
effects = []
simulation = "pure"
determinism = "deterministic"
"#,
    );
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "right-version but wrong-hash declarations must not be trusted",
    );
    assert_contains(
        &output.combined(),
        "summary_hash",
        "declaration hash mismatch should name the hash field",
    );
}

#[test]
fn stale_declaration_version_emits_k0121() {
    let project = TestProject::new("v11-declaration-stale");
    write_sqlx_project(&project);
    valid_declaration(&project, "0.7");
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "stale declaration must fail typed replay-critical check",
    );
    assert_contains(
        &output.combined(),
        "K0121",
        "stale declaration should use declaration validation diagnostic",
    );
}

#[test]
fn activity_declaration_without_retry_emits_k0125() {
    let project = TestProject::new("v11-declaration-activity-retry");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "activity_decl"
version = "0.1.0"
edition = "2021"

[dependencies]
payments = "0.1"
"#,
    );
    project.write(
        "payments.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "payments"
version = "0.1"
source = "bindgen"

[[activity]]
path = "payments::charge"
result = "record"
retry = "retry-safe"
idempotency = "request-id"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "payments", policy = "activity", reason = "external payment activity")]
use payments::charge;

fn main() {
    let _ = charge();
}
"#,
    );

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "activity retry metadata issue should be visible without blocking normal check",
    );
    assert_contains(
        &output.stdout,
        "K0125",
        "activity declaration without retry metadata should emit K0125",
    );
}

#[test]
fn lsp_declaration_diagnostics_include_k012x_actions() {
    let project = TestProject::new("v11-declaration-lsp-k012x");
    write_sqlx_project(&project);
    let missing_file = typed_sqlx_file(&project);

    let missing = run_kobo(
        &[
            s("lsp-diagnostics"),
            path_arg(&missing_file),
            s("--include-actions"),
        ],
        &project.root,
    );
    assert_success(
        &missing,
        "LSP diagnostics should render typed declaration gaps",
    );
    assert_contains(
        &missing.stdout,
        "K0122",
        "missing typed declaration should surface through LSP diagnostics",
    );
    assert_contains(
        &missing.stdout,
        "Create declaration file",
        "LSP diagnostics should offer declaration-specific code actions",
    );

    valid_declaration(&project, "0.7");
    let stale = run_kobo(
        &[
            s("lsp-diagnostics"),
            path_arg(&missing_file),
            s("--include-actions"),
        ],
        &project.root,
    );
    assert_success(&stale, "LSP diagnostics should render stale declarations");
    assert_contains(
        &stale.stdout,
        "K0121",
        "stale typed declaration should surface through LSP diagnostics",
    );
    assert_contains(
        &stale.stdout,
        "Validate declaration file",
        "stale declarations should have declaration validation actions",
    );

    project.write(
        "sqlx.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"
source = "bindgen"

[[function]]
path = "sqlx::Pool::other_call"
effects = []
simulation = "pure"
determinism = "deterministic"
"#,
    );
    let wrong_call = run_kobo(
        &[
            s("lsp-diagnostics"),
            path_arg(&missing_file),
            s("--include-actions"),
        ],
        &project.root,
    );
    assert_success(
        &wrong_call,
        "LSP diagnostics should render wrong-call typed declarations",
    );
    assert_contains(
        &wrong_call.stdout,
        "does not cover boundary call",
        "LSP should mirror check's typed call coverage validation",
    );

    project.write(
        "sqlx.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "sqlx"
version = "0.8"
source = "bindgen"

[[function]]
path = "sqlx::Pool::connect_lazy"
effects = ["database"]
simulation = "record"
"#,
    );
    let effectful = run_kobo(
        &[
            s("lsp-diagnostics"),
            path_arg(&missing_file),
            s("--include-actions"),
        ],
        &project.root,
    );
    assert_success(
        &effectful,
        "LSP diagnostics should render effectful typed declarations",
    );
    assert_contains(
        &effectful.stdout,
        "still has replay-critical effects",
        "LSP should mirror check's typed effect validation",
    );
}

#[test]
fn lsp_activity_diagnostics_are_call_specific_for_methods() {
    let project = TestProject::new("v11-declaration-lsp-method-activity");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "method_activity"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = "0.12"
"#,
    );
    project.write(
        "reqwest.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "reqwest"
version = "0.12"
source = "bindgen"

[[activity]]
path = "reqwest::Client::send"
result = "record"
retry = "retry-safe"
idempotency = "request-id"
compensation = "cancel-request"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "activity", reason = "external request activity")]
use reqwest::Client;

fn main() {
    let _ = Client::new().send();
}
"#,
    );

    let complete = run_kobo(
        &[
            s("lsp-diagnostics"),
            path_arg(&file),
            s("--include-actions"),
        ],
        &project.root,
    );
    assert_success(
        &complete,
        "LSP should accept activity metadata for the exact external method call",
    );
    assert!(
        !complete.stdout.contains("K0125"),
        "complete metadata for reqwest::Client::send should not emit K0125:\n{}",
        complete.stdout
    );

    project.write(
        "reqwest.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "reqwest"
version = "0.12"
source = "bindgen"

[[activity]]
path = "reqwest::Client::new"
result = "record"
retry = "retry-safe"
idempotency = "request-id"
compensation = "cancel-request"
"#,
    );
    let wrong_call = run_kobo(
        &[
            s("lsp-diagnostics"),
            path_arg(&file),
            s("--include-actions"),
        ],
        &project.root,
    );
    assert_success(
        &wrong_call,
        "LSP should render activity diagnostics for wrong-call metadata",
    );
    assert_contains(
        &wrong_call.stdout,
        "K0125",
        "activity metadata must cover the exact method call, not only the crate",
    );
}

#[test]
fn declaration_parse_error_points_to_file_and_key() {
    let project = TestProject::new("v11-declaration-parse-error");
    write_sqlx_project(&project);
    project.write("sqlx.kobo.d.toml", "[crate]\nname = \n");
    let file = typed_sqlx_file(&project);

    let output = run_kobo(
        &[
            s("check"),
            path_arg(&file),
            s("--replay-critical"),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(&output, "invalid declaration TOML must fail");
    assert_contains(&output.combined(), "K0121", "parse error should use K0121");
    assert_contains(
        &output.combined(),
        "sqlx.kobo.d.toml",
        "diagnostic should name the declaration file",
    );
    assert_contains(
        &output.combined(),
        "crate.name",
        "diagnostic should name the declaration key being validated",
    );
}

#[test]
fn declaration_hash_is_serialized_in_kwit() {
    let project = TestProject::new("v11-declaration-witness-hash");
    project.write(
        "Cargo.toml",
        r#"[package]
name = "decl_witness"
version = "0.1.0"
edition = "2021"
"#,
    );
    project.write(
        "reqwest.kobo.d.toml",
        r#"schema_version = 0

[crate]
name = "reqwest"
version = "0.12"
source = "bindgen"

[[function]]
path = "reqwest::Client::new"
effects = ["network"]
simulation = "record"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::boundary(crate = "reqwest", policy = "typed", reason = "typed declaration supplied")]
use reqwest::Client;

#[kobo::scenario(profile = "async")]
fn typed_gateway() {
    let _client = Client::new();
    ward.task();
}
"#,
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
    assert_success(
        &output,
        "typed declaration scenario should produce a witness",
    );
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    assert_contains(
        &witness.to_string(),
        "declaration_hash",
        "witness should serialize declaration hash/version evidence",
    );
    assert_contains(
        &witness.to_string(),
        r#""schema_version":0"#,
        "witness should serialize contract-shaped declaration schema version",
    );
    assert_contains(
        &witness.to_string(),
        r#""hash""#,
        "witness should serialize contract-shaped declaration hash",
    );
    assert_contains(
        &witness["ecosystem_boundaries"].to_string(),
        r#""declaration""#,
        "boundary evidence should include declaration metadata",
    );
    assert_contains(
        &witness.to_string(),
        "reqwest.kobo.d.toml",
        "witness should serialize declaration source path",
    );
}

#[test]
fn active_k012x_codes_have_explain_text() {
    let project = TestProject::new("v11-k012x-explain");
    for code in [
        "K0120", "K0121", "K0122", "K0123", "K0124", "K0125", "K0126", "K0127", "K0128", "K0129",
    ] {
        let output = run_kobo(&[s("explain"), s(code)], &project.root);
        assert_success(&output, "active K012x code should be explainable");
        assert_contains(
            &output.stdout,
            code,
            "explain output should include the diagnostic code",
        );
        assert!(
            !output.stdout.contains("reserved Kobo diagnostic slot"),
            "{code} must be active, not a reserved placeholder:\n{}",
            output.stdout
        );
    }
}
