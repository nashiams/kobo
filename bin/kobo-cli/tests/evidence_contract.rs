use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct FixtureCase {
    root: PathBuf,
    fixture_path: PathBuf,
}

impl FixtureCase {
    fn new(name: &str, fixture_name: &str) -> Self {
        let workspace_root = workspace_root();
        let unique_id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = workspace_root
            .join("target-test-fixtures")
            .join(format!("{name}-{}-{unique_id}", std::process::id()));

        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }

        fs::create_dir_all(&root).expect("temp fixture dir should be creatable");

        let source_fixture = workspace_root
            .join("tests")
            .join("fixtures")
            .join(fixture_name);
        let fixture_path = root.join(fixture_name);
        fs::copy(&source_fixture, &fixture_path).expect("fixture should copy");

        Self { root, fixture_path }
    }

    fn overwrite_source(&self, source: &str) {
        fs::write(&self.fixture_path, source).expect("fixture source should be writable");
    }
}

impl Drop for FixtureCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct KoboOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SolverEvidenceSummary {
    outcome: String,
    graph_fingerprint: String,
    node_count: u64,
    edge_count: u64,
}

#[test]
fn inspect_must_expose_structured_solver_evidence_from_cli_path() {
    let case = FixtureCase::new("evidence-contract-inspect", "tiered_mix.kobo");
    let (_output, source_map) = inspect_source_map(&case);

    assert_source_map_has_solver_evidence(&source_map);
}

#[test]
fn solver_graph_fingerprint_must_change_when_cli_input_changes() {
    let base_case = FixtureCase::new("evidence-contract-base", "tiered_mix.kobo");
    let variant_case = FixtureCase::new("evidence-contract-variant", "tiered_mix.kobo");

    let original = fs::read_to_string(&variant_case.fixture_path).expect("fixture should read");
    let variant = original.replace(
        "    let names: Vec<String> = Vec::new();",
        "    let oracle_nonce = String::from(\"verification-oracle\");\n    println!(\"{}\", oracle_nonce);\n\n    let names: Vec<String> = Vec::new();",
    );
    assert_ne!(
        original, variant,
        "fixture mutation must change the CLI input"
    );
    variant_case.overwrite_source(&variant);

    let (_base_output, base_map) = inspect_source_map(&base_case);
    let (_variant_output, variant_map) = inspect_source_map(&variant_case);
    let base_fingerprint = assert_source_map_has_solver_evidence(&base_map);
    let variant_fingerprint = assert_source_map_has_solver_evidence(&variant_map);

    assert_ne!(
        base_fingerprint, variant_fingerprint,
        "solver graph fingerprint must be derived from the actual CLI input, not a constant"
    );
}

#[test]
fn migrate_dry_run_must_expose_solver_evidence_matching_inspect() {
    let case = FixtureCase::new("evidence-contract-migrate", "tiered_mix.kobo");
    let (_inspect_output, source_map) = inspect_source_map(&case);
    let expected = source_map_solver_evidence_summary(&source_map);

    let migrate_output = run_kobo(["migrate", "--dry-run"], &case.fixture_path);
    assert_migrate_output_matches_solver_evidence(&migrate_output, &expected);
}

#[test]
fn migrate_solver_evidence_must_change_when_cli_input_changes() {
    let base_case = FixtureCase::new("evidence-contract-migrate-base", "tiered_mix.kobo");
    let variant_case = FixtureCase::new("evidence-contract-migrate-variant", "tiered_mix.kobo");

    let original = fs::read_to_string(&variant_case.fixture_path).expect("fixture should read");
    let variant = original.replace(
        "    let names: Vec<String> = Vec::new();",
        "    let oracle_nonce = String::from(\"verification-oracle\");\n    println!(\"{}\", oracle_nonce);\n\n    let names: Vec<String> = Vec::new();",
    );
    assert_ne!(
        original, variant,
        "fixture mutation must change the CLI input"
    );
    variant_case.overwrite_source(&variant);

    let (_base_inspect, base_map) = inspect_source_map(&base_case);
    let (_variant_inspect, variant_map) = inspect_source_map(&variant_case);
    let base_expected = source_map_solver_evidence_summary(&base_map);
    let variant_expected = source_map_solver_evidence_summary(&variant_map);
    assert_ne!(
        base_expected.graph_fingerprint, variant_expected.graph_fingerprint,
        "independent inspect evidence must prove the two inputs produce different graphs"
    );

    let base_migrate = run_kobo(["migrate", "--dry-run"], &base_case.fixture_path);
    let variant_migrate = run_kobo(["migrate", "--dry-run"], &variant_case.fixture_path);
    let base_seen = assert_migrate_output_matches_solver_evidence(&base_migrate, &base_expected);
    let variant_seen =
        assert_migrate_output_matches_solver_evidence(&variant_migrate, &variant_expected);

    assert_ne!(
        base_seen, variant_seen,
        "migrate dry-run evidence must vary with input instead of using a hardcoded fingerprint"
    );
}

fn inspect_source_map(case: &FixtureCase) -> (KoboOutput, Value) {
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );

    let source_map = fs::read_to_string(case.fixture_path.with_extension("kobo.map"))
        .expect("inspect should emit a source map");
    let parsed = serde_json::from_str(&source_map).unwrap_or_else(|error| {
        panic!("source map must be valid JSON: {error}\nsource map:\n{source_map}")
    });

    (output, parsed)
}

fn assert_source_map_has_solver_evidence(source_map: &Value) -> String {
    let summary = source_map_solver_evidence_summary(source_map);
    let evidence = source_map
        .get("solver_evidence")
        .and_then(Value::as_object)
        .expect("source map must contain top-level solver_evidence object");

    let budget = evidence
        .get("budget")
        .and_then(Value::as_object)
        .expect("solver_evidence.budget must be an object");
    assert!(
        budget
            .get("max_cluster_size")
            .and_then(Value::as_u64)
            .is_some(),
        "solver evidence must expose max_cluster_size"
    );
    assert!(
        budget
            .get("budget_seconds")
            .and_then(Value::as_f64)
            .is_some(),
        "solver evidence must expose budget_seconds"
    );

    let mappings = source_map
        .get("x_kobo_mappings")
        .and_then(Value::as_array)
        .expect("source map must contain x_kobo_mappings array");
    assert!(
        !mappings.is_empty(),
        "fixture should produce mapped bindings"
    );
    assert!(
        summary.node_count >= mappings.len() as u64,
        "solver evidence node_count must cover mapped bindings"
    );

    let mut node_ids = BTreeSet::new();
    for entry in mappings {
        assert!(
            entry
                .get("ownership_tier")
                .and_then(Value::as_str)
                .is_some(),
            "mapping must retain ownership_tier evidence"
        );
        let solver_outcome = entry
            .get("solver_outcome")
            .and_then(Value::as_str)
            .expect("each mapping must carry solver_outcome provenance");
        assert_eq!(
            solver_outcome,
            summary.outcome.as_str(),
            "mapping outcome must match top-level solver outcome"
        );
        let decision_source = entry
            .get("decision_source")
            .and_then(Value::as_str)
            .expect("each mapping must carry decision_source provenance");
        assert_eq!(
            decision_source, "solver",
            "mapped ownership tiers must come from the real solver path"
        );
        let node_id = entry
            .get("solver_node_id")
            .and_then(Value::as_u64)
            .expect("each mapping must carry solver_node_id provenance");
        node_ids.insert(node_id);
    }

    assert!(
        node_ids.len() >= 2,
        "solver_node_id provenance must not be a single constant for every mapping"
    );

    summary.graph_fingerprint
}

fn source_map_solver_evidence_summary(source_map: &Value) -> SolverEvidenceSummary {
    let evidence = source_map
        .get("solver_evidence")
        .and_then(Value::as_object)
        .expect("source map must contain top-level solver_evidence object");

    let outcome = evidence
        .get("outcome")
        .and_then(Value::as_str)
        .expect("solver_evidence.outcome must be a string");
    assert!(
        matches!(
            outcome,
            "Unique"
                | "MultiSolution"
                | "NoSolution"
                | "ClusterTooLarge"
                | "BudgetExceeded"
                | "BoundaryStop"
        ),
        "unexpected solver outcome evidence: {outcome}"
    );

    let graph_fingerprint = evidence
        .get("graph_fingerprint")
        .and_then(Value::as_str)
        .expect("solver_evidence.graph_fingerprint must be a string");
    assert!(
        graph_fingerprint.len() >= 16,
        "graph fingerprint must be long enough to not be decorative"
    );

    let node_count = evidence
        .get("node_count")
        .and_then(Value::as_u64)
        .expect("solver_evidence.node_count must be a number");
    let edge_count = evidence
        .get("edge_count")
        .and_then(Value::as_u64)
        .expect("solver_evidence.edge_count must be a number");

    SolverEvidenceSummary {
        outcome: outcome.to_owned(),
        graph_fingerprint: graph_fingerprint.to_owned(),
        node_count,
        edge_count,
    }
}

fn assert_migrate_output_matches_solver_evidence(
    output: &KoboOutput,
    expected: &SolverEvidenceSummary,
) -> String {
    assert!(
        output.status.success(),
        "migrate dry-run must succeed before evidence can be trusted\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );

    let combined = format!("{}\n{}", output.stdout, output.stderr);
    assert!(
        combined.contains("solver"),
        "migrate dry-run must expose solver evidence, not only greedy text\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert!(
        combined.contains(&expected.outcome),
        "migrate dry-run outcome must match inspect source-map evidence `{}`\nstdout:\n{}\nstderr:\n{}",
        expected.outcome,
        output.stdout,
        output.stderr
    );
    assert!(
        combined.contains(&expected.graph_fingerprint),
        "migrate dry-run fingerprint must match inspect source-map evidence `{}`\nstdout:\n{}\nstderr:\n{}",
        expected.graph_fingerprint,
        output.stdout,
        output.stderr
    );
    assert!(
        combined.contains(&expected.node_count.to_string()),
        "migrate dry-run must expose solver node count `{}`\nstdout:\n{}\nstderr:\n{}",
        expected.node_count,
        output.stdout,
        output.stderr
    );
    assert!(
        combined.contains(&expected.edge_count.to_string()),
        "migrate dry-run must expose solver edge count `{}`\nstdout:\n{}\nstderr:\n{}",
        expected.edge_count,
        output.stdout,
        output.stderr
    );

    expected.graph_fingerprint.clone()
}

fn run_kobo<const N: usize>(args: [&str; N], fixture_path: &Path) -> KoboOutput {
    let workspace_root = workspace_root();
    let relative_fixture = fixture_path
        .strip_prefix(&workspace_root)
        .expect("fixture should live under workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(args)
        .arg(relative_fixture)
        .current_dir(&workspace_root)
        .output()
        .expect("kobo command should run");

    KoboOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}
