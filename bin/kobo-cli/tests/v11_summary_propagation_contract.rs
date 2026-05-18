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

#[test]
fn build_emits_kobo_summary_and_downstream_rejects_mutated_hash() {
    let upstream = TestProject::new("v11-summary-upstream");
    upstream.write(
        "Cargo.toml",
        r#"[package]
name = "upstream_kobo"
version = "0.1.0"
edition = "2021"
"#,
    );
    upstream.main_file(
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {}

fn main() {
    let _tx = Transaction {};
}
"#,
    );
    let build = run_kobo(&[s("build")], &upstream.root);
    assert_success(&build, "upstream build should emit a .kobo-summary");

    let summary_path = upstream.root.join("target/kobo-gen/.kobo-summary");
    let summary: Value = serde_json::from_str(
        &fs::read_to_string(&summary_path).expect(".kobo-summary should be readable"),
    )
    .expect(".kobo-summary should be JSON");
    assert_eq!(summary["schema_version"], 1);
    assert_contains(
        &summary.to_string(),
        "Transaction",
        "summary should carry Kobo-authored obligation facts",
    );
    assert_contains(
        &summary.to_string(),
        "upstream_kobo::main",
        "summary obligation facts should retain function applicability",
    );
    assert_contains(
        &summary.to_string(),
        "applicable_types",
        "summary obligation facts should retain type applicability",
    );
    assert_contains(
        &summary["obligations"][0]["applicable_types"].to_string(),
        "upstream_kobo::Transaction",
        "summary applicability should qualify owner types with the producing crate",
    );

    let summary_hash = summary["summary_hash"]
        .as_str()
        .expect("summary should contain a hash");
    let summary_consumer = TestProject::new("v11-summary-liveness-consumer");
    summary_consumer.main_file(
        r#"
use upstream_kobo::Transaction;

fn main() {
    let _external: Option<Transaction> = None;
}
"#,
    );
    summary_consumer.write(
        "Kobo.toml",
        &format!(
            r#"[ecosystem]
default = "opaque"

[[ecosystem.summary]]
crate = "upstream_kobo"
path = "{}"
hash = "{summary_hash}"
"#,
            summary_path.display().to_string().replace('\\', "/")
        ),
    );
    let debt = run_kobo(
        &[
            s("debt"),
            path_arg(&summary_consumer.root.join("src/main.kobo")),
            s("--liveness"),
            s("--json"),
        ],
        &summary_consumer.root,
    );
    assert_success(
        &debt,
        "downstream liveness debt should consume valid .kobo-summary facts",
    );
    assert_contains(
        &debt.stdout,
        "Transaction",
        "summary obligation should affect downstream liveness analysis",
    );
    assert_contains(
        &debt.stdout,
        "upstream_kobo::main",
        "downstream debt output should preserve summary function applicability",
    );
    assert_contains(
        &debt.stdout,
        "applicability",
        "downstream debt output should preserve structured applicability",
    );

    let unrelated_consumer = TestProject::new("v11-summary-unrelated-consumer");
    unrelated_consumer.main_file("fn main() {}\n");
    unrelated_consumer.write(
        "Kobo.toml",
        &format!(
            r#"[ecosystem]
default = "opaque"

[[ecosystem.summary]]
crate = "upstream_kobo"
path = "{}"
hash = "{summary_hash}"
"#,
            summary_path.display().to_string().replace('\\', "/")
        ),
    );
    let unrelated_debt = run_kobo(
        &[
            s("debt"),
            path_arg(&unrelated_consumer.root.join("src/main.kobo")),
            s("--liveness"),
            s("--json"),
        ],
        &unrelated_consumer.root,
    );
    assert_success(
        &unrelated_debt,
        "unrelated downstream source should still accept valid summary metadata",
    );
    assert!(
        !unrelated_debt.stdout.contains("Transaction"),
        "summary obligations should not apply when downstream source references no applicable type/function:\n{}",
        unrelated_debt.stdout
    );

    let text_only_consumer = TestProject::new("v11-summary-text-only-consumer");
    text_only_consumer.main_file(
        r#"
// Transaction and upstream_kobo::main appear only in comments.
fn main() {
    let _message = "Transaction upstream_kobo::main";
    struct Transaction;
}
"#,
    );
    text_only_consumer.write(
        "Kobo.toml",
        &format!(
            r#"[ecosystem]
default = "opaque"

[[ecosystem.summary]]
crate = "upstream_kobo"
path = "{}"
hash = "{summary_hash}"
"#,
            summary_path.display().to_string().replace('\\', "/")
        ),
    );
    let text_only_debt = run_kobo(
        &[
            s("debt"),
            path_arg(&text_only_consumer.root.join("src/main.kobo")),
            s("--liveness"),
            s("--json"),
        ],
        &text_only_consumer.root,
    );
    assert_success(
        &text_only_debt,
        "comments, strings, and local type names should not activate external summary applicability",
    );
    assert!(
        !text_only_debt.stdout.contains("upstream_kobo::main"),
        "summary applicability should be based on references, not raw text:\n{}",
        text_only_debt.stdout
    );

    let local_type_consumer = TestProject::new("v11-summary-local-type-consumer");
    local_type_consumer.main_file(
        r#"
fn main() {
    struct Transaction;
    let _local: Option<Transaction> = None;
}
"#,
    );
    local_type_consumer.write(
        "Kobo.toml",
        &format!(
            r#"[ecosystem]
default = "opaque"

[[ecosystem.summary]]
crate = "upstream_kobo"
path = "{}"
hash = "{summary_hash}"
"#,
            summary_path.display().to_string().replace('\\', "/")
        ),
    );
    let local_type_debt = run_kobo(
        &[
            s("debt"),
            path_arg(&local_type_consumer.root.join("src/main.kobo")),
            s("--liveness"),
            s("--json"),
        ],
        &local_type_consumer.root,
    );
    assert_success(
        &local_type_debt,
        "local same-name type references should not activate external summary applicability",
    );
    assert!(
        !local_type_debt.stdout.contains("upstream_kobo::main"),
        "local same-name type references should not activate upstream summary facts:\n{}",
        local_type_debt.stdout
    );

    let other_crate_consumer = TestProject::new("v11-summary-other-crate-type-consumer");
    other_crate_consumer.main_file(
        r#"
use other_kobo::Transaction;

fn main() {
    let _external: Option<Transaction> = None;
}
"#,
    );
    other_crate_consumer.write(
        "Kobo.toml",
        &format!(
            r#"[ecosystem]
default = "opaque"

[[ecosystem.summary]]
crate = "upstream_kobo"
path = "{}"
hash = "{summary_hash}"
"#,
            summary_path.display().to_string().replace('\\', "/")
        ),
    );
    let other_crate_debt = run_kobo(
        &[
            s("debt"),
            path_arg(&other_crate_consumer.root.join("src/main.kobo")),
            s("--liveness"),
            s("--json"),
        ],
        &other_crate_consumer.root,
    );
    assert_success(
        &other_crate_debt,
        "other-crate same-name references should not activate external summary applicability",
    );
    assert!(
        !other_crate_debt.stdout.contains("upstream_kobo::main"),
        "other-crate same-name references should not activate upstream summary facts:\n{}",
        other_crate_debt.stdout
    );

    let downstream = TestProject::new("v11-summary-downstream");
    downstream.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "downstream_kobo"
version = "0.1.0"
edition = "2021"

[dependencies]
upstream_kobo = {{ path = "{}" }}
"#,
            upstream.root.display().to_string().replace('\\', "/")
        ),
    );
    downstream.main_file("fn main() {}\n");
    downstream.write(
        "Kobo.toml",
        &format!(
            r#"[ecosystem]
default = "opaque"

[[ecosystem.summary]]
crate = "upstream_kobo"
path = "{}"
hash = "stale-hash"
"#,
            summary_path.display().to_string().replace('\\', "/")
        ),
    );

    let inspect = run_kobo(
        &[
            s("inspect"),
            path_arg(&downstream.root.join("src/main.kobo")),
        ],
        &downstream.root,
    );
    assert_failure(
        &inspect,
        "downstream analysis must reject or downgrade stale .kobo-summary hashes",
    );
    assert_contains(
        &inspect.combined(),
        "K0126",
        "summary hash mismatch should use the v0.11 summary diagnostic",
    );

    let stale_debt = run_kobo(
        &[
            s("debt"),
            path_arg(&downstream.root.join("src/main.kobo")),
            s("--liveness"),
            s("--json"),
        ],
        &downstream.root,
    );
    assert_failure(
        &stale_debt,
        "debt --liveness must validate summary hashes before consuming facts",
    );
    assert_contains(
        &stale_debt.combined(),
        "K0126",
        "stale summary should not be trusted by debt analysis",
    );
}
