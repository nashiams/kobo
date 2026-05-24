//! RED oracle tests for the v0.8.5 roadmap.
//!
//! These tests intentionally exercise public CLI behavior before the feature
//! work exists. They should fail until each phase is implemented through the
//! spec in `.claude/prompt/roadmap/v0.8.5/acceptance/oracle_gates.md`.
//!
//! Keep these tests independent from helper internals: a later implementation
//! agent should have to satisfy real command behavior, source sensitivity, and
//! fixture variants rather than matching one hardcoded string.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    file: PathBuf,
}

impl Fixture {
    fn new(label: &str, file_name: &str, source: &str) -> Self {
        let root = unique_temp_root(label);
        fs::create_dir_all(&root).expect("fixture root should be creatable");
        let file = root.join(file_name);
        fs::write(&file, source).expect("fixture source should be writable");
        Self { root, file }
    }

    fn project(label: &str, files: &[(&str, &str)]) -> Self {
        let root = unique_temp_root(label);
        for (relative, contents) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("fixture parent should be creatable");
            }
            fs::write(path, contents).expect("fixture project file should be writable");
        }
        let file = root.join("src/main.kobo");
        Self { root, file }
    }

    fn read_source(&self) -> String {
        fs::read_to_string(&self.file).expect("fixture should read")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct CliOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl CliOutput {
    fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn unique_temp_root(label: &str) -> PathBuf {
    let counter = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}-{}-{counter}", std::process::id()))
}

fn run_kobo(args: &[String]) -> CliOutput {
    run_kobo_in(args, None)
}

fn run_kobo_in(args: &[String], cwd: Option<&Path>) -> CliOutput {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kobo"));
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().expect("kobo command should launch");
    CliOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn arg(value: impl AsRef<Path>) -> String {
    value.as_ref().to_string_lossy().into_owned()
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn args_with_file(values: &[&str], file: &Path) -> Vec<String> {
    let mut out = args(values);
    out.push(arg(file));
    out
}

fn assert_success(output: &CliOutput, context: &str) {
    assert!(
        output.status.success(),
        "{context} must succeed\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

fn assert_failure(output: &CliOutput, context: &str) {
    assert!(
        !output.status.success(),
        "{context} must fail\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

fn assert_contains_all(text: &str, needles: &[&str], context: &str) {
    for needle in needles {
        assert!(
            text.contains(needle),
            "{context}\nmissing `{needle}` in:\n{text}"
        );
    }
}

fn assert_not_contains(text: &str, needle: &str, context: &str) {
    assert!(
        !text.contains(needle),
        "{context}\nunexpected `{needle}` in:\n{text}"
    );
}

fn diagnostic_json_values(output: &CliOutput) -> Vec<Value> {
    output
        .combined()
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('{') {
                serde_json::from_str::<Value>(trimmed).ok()
            } else {
                None
            }
        })
        .collect()
}

fn diagnostic_codes(output: &CliOutput) -> Vec<String> {
    diagnostic_json_values(output)
        .into_iter()
        .filter_map(|value| value["code"].as_str().map(ToOwned::to_owned))
        .collect()
}

fn count_code(codes: &[String], code: &str) -> usize {
    codes
        .iter()
        .filter(|actual| actual.as_str() == code)
        .count()
}

fn assert_json_identity(values: &[Value], context: &str) {
    assert!(!values.is_empty(), "{context}: expected JSON diagnostics");
    for value in values {
        for field in [
            "schema_version",
            "code",
            "slug",
            "title",
            "severity",
            "primary",
        ] {
            assert!(
                value.get(field).is_some(),
                "{context}: diagnostic JSON missing `{field}`:\n{value}"
            );
        }
    }
}

fn first_named_line(text: &str, prefix: &str) -> String {
    text.lines()
        .find(|line| line.trim_start().starts_with(prefix))
        .unwrap_or_else(|| panic!("missing `{prefix}` line in:\n{text}"))
        .trim()
        .to_owned()
}

#[test]
fn phase_00_explain_and_json_share_registry_identity() {
    let boundary = run_kobo(&args(&["explain", "K0107", "--verbose"]));
    let suppression = run_kobo(&args(&["explain", "K0108", "--verbose"]));
    assert_success(&boundary, "phase 00 K0107 explain");
    assert_success(&suppression, "phase 00 K0108 explain");

    let boundary_text = boundary.combined();
    let suppression_text = suppression.combined();
    assert_contains_all(
        &boundary_text,
        &[
            "K0107",
            "slug: unmodeled-external-boundary",
            "mode policy:",
            "normal Rust crates remain allowed",
        ],
        "phase 00 K0107 explain must come from the registry contract",
    );
    assert_contains_all(
        &suppression_text,
        &[
            "K0108",
            "slug: replay-obligation-suppressed",
            "reason",
            "reviewable evidence",
        ],
        "phase 00 K0108 explain must come from the registry contract",
    );
    assert_ne!(
        boundary_text, suppression_text,
        "phase 00 explain output must be code-specific, not a constant card"
    );

    let fixture = Fixture::new("v085-phase00-json", "bad.kobo", "fn main() { let x = }\n");
    let json = run_kobo(&args_with_file(
        &["check", "--recover-parse", "--error-format=json"],
        &fixture.file,
    ));
    let diagnostics = diagnostic_json_values(&json);
    assert_json_identity(
        &diagnostics,
        "phase 00 diagnostic JSON must expose registry identity fields",
    );
}

#[test]
fn phase_01_recovery_and_query_foundation_are_content_sensitive() {
    let mixed = Fixture::new(
        "v085-phase01-mixed",
        "mixed.kobo",
        r#"
fn broken() {
    let x =
}

fn good() {
    let name = String::from("alpha");
    consume(name);
    println!("{}", name);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );
    let fixed_non_poisoned = Fixture::new(
        "v085-phase01-fixed",
        "fixed.kobo",
        r#"
fn broken() {
    let x =
}

fn good() {
    let name = String::from("alpha");
    println!("{}", name);
}
"#,
    );

    let mixed_out = run_kobo(&args_with_file(
        &[
            "check",
            "--checked",
            "--recover-parse",
            "--error-format=json",
        ],
        &mixed.file,
    ));
    let fixed_out = run_kobo(&args_with_file(
        &[
            "check",
            "--checked",
            "--recover-parse",
            "--error-format=json",
        ],
        &fixed_non_poisoned.file,
    ));
    let mixed_codes = diagnostic_codes(&mixed_out);
    let fixed_codes = diagnostic_codes(&fixed_out);

    assert!(
        mixed_codes.iter().any(|code| code.starts_with("K011")),
        "phase 01 parser recovery must use non-conflicting K011x codes, got {mixed_codes:?}\n{}",
        mixed_out.combined()
    );
    assert!(
        !mixed_codes
            .iter()
            .any(|code| matches!(code.as_str(), "K0100" | "K0101" | "K0102" | "K0107" | "K0108")),
        "phase 01 parser recovery must not consume v0.8.5 liveness/replay code space: {mixed_codes:?}"
    );
    assert!(
        mixed_codes.iter().any(|code| code == "K0001"),
        "phase 01 recovery must continue trustworthy analysis outside poisoned regions:\n{}",
        mixed_out.combined()
    );
    assert!(
        !fixed_codes.iter().any(|code| code == "K0001"),
        "phase 01 query/output must change when the non-poisoned source changes:\n{}",
        fixed_out.combined()
    );
    assert_json_identity(
        &diagnostic_json_values(&mixed_out),
        "phase 01 JSON diagnostics must include registry identity",
    );
}

#[test]
fn phase_02_lsp_payload_export_matches_cli_json() {
    let fixture = Fixture::new(
        "v085-phase02-lsp",
        "lsp.kobo",
        r#"
fn main() {
    let config = String::from("alpha");
    consume(config);
    println!("{}", config);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );

    let cli = run_kobo(&args_with_file(
        &["check", "--checked", "--error-format=json"],
        &fixture.file,
    ));
    let lsp = run_kobo(&args_with_file(
        &["lsp-diagnostics", "--format=json"],
        &fixture.file,
    ));
    let cli_codes = diagnostic_codes(&cli);
    let lsp_values = diagnostic_json_values(&lsp);
    let lsp_codes = lsp_values
        .iter()
        .filter_map(|value| value["code"].as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();

    assert_failure(&cli, "phase 02 CLI fixture must produce a diagnostic");
    assert_success(&lsp, "phase 02 LSP diagnostic export");
    assert!(
        cli_codes.iter().any(|code| code == "K0001"),
        "phase 02 CLI baseline must include K0001, got {cli_codes:?}"
    );
    assert_eq!(
        cli_codes, lsp_codes,
        "phase 02 LSP payload must preserve the same diagnostic codes as CLI JSON"
    );
    for value in lsp_values {
        assert_contains_all(
            &value.to_string(),
            &[
                "\"uri\"",
                "\"range\"",
                "\"slug\"",
                "\"title\"",
                "kobo explain",
            ],
            "phase 02 LSP diagnostics need editor-ready identity and explain action",
        );
    }
}

#[test]
fn phase_03_must_call_metadata_reaches_inspect_without_enforcement() {
    let commit_or_rollback = Fixture::new(
        "v085-phase03-commit-rollback",
        "transaction.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {
    id: u64,
}

fn create() -> Transaction {
    Transaction { id: 1 }
}

fn main() {
    let tx = create();
    println!("{}", tx.id);
}
"#,
    );
    let finish_or_cancel = Fixture::new(
        "v085-phase03-finish-cancel",
        "operation.kobo",
        r#"
#[kobo::must_call(finish | cancel)]
struct Operation {
    id: u64,
}

fn create() -> Operation {
    Operation { id: 1 }
}

fn main() {
    let op = create();
    println!("{}", op.id);
}
"#,
    );

    let tx = run_kobo(&args_with_file(&["inspect"], &commit_or_rollback.file));
    let op = run_kobo(&args_with_file(&["inspect"], &finish_or_cancel.file));
    assert_success(&tx, "phase 03 inspect must accept must_call metadata");
    assert_success(
        &op,
        "phase 03 inspect must accept alternate must_call obligations",
    );
    assert_contains_all(
        &tx.combined(),
        &["must_call", "commit", "rollback"],
        "phase 03 inspect output must expose obligation metadata",
    );
    assert_contains_all(
        &op.combined(),
        &["must_call", "finish", "cancel"],
        "phase 03 metadata must be source-sensitive",
    );
    assert_not_contains(
        &tx.combined(),
        "K0100",
        "phase 03 must not enforce unresolved obligations before phase 04",
    );
    assert_ne!(
        tx.combined(),
        op.combined(),
        "phase 03 metadata output cannot be hardcoded to one obligation set"
    );
}

#[test]
fn phase_04_liveness_debt_distinguishes_unresolved_commit_and_rollback() {
    let unresolved = Fixture::new(
        "v085-phase04-unresolved",
        "unresolved.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {
    id: u64,
}

impl Transaction {
    fn commit(self) {}
    fn rollback(self) {}
}

fn handler(fail: bool) {
    let tx = Transaction { id: 7 };
    if fail {
        return;
    }
    tx.commit();
}
"#,
    );
    let committed = Fixture::new(
        "v085-phase04-committed",
        "committed.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {
    id: u64,
}

impl Transaction {
    fn commit(self) {}
    fn rollback(self) {}
}

fn handler(fail: bool) {
    let tx = Transaction { id: 7 };
    if fail {
        tx.rollback();
        return;
    }
    tx.commit();
}
"#,
    );

    let unresolved_out = run_kobo(&args_with_file(&["debt", "--liveness"], &unresolved.file));
    let committed_out = run_kobo(&args_with_file(&["debt", "--liveness"], &committed.file));
    assert_success(
        &unresolved_out,
        "phase 04 debt --liveness unresolved fixture",
    );
    assert_success(&committed_out, "phase 04 debt --liveness resolved fixture");
    assert_contains_all(
        &unresolved_out.combined(),
        &["K0100", "Transaction", "commit", "rollback"],
        "phase 04 unresolved path must emit obligation-specific K0100",
    );
    assert_not_contains(
        &committed_out.combined(),
        "K0100",
        "phase 04 resolved rollback/commit paths must remove K0100",
    );
    assert_ne!(
        unresolved_out.combined(),
        committed_out.combined(),
        "phase 04 liveness report must be path-sensitive"
    );
}

#[test]
fn phase_05_sim_scout_ranking_changes_with_project_signals() {
    let backend = Fixture::new(
        "v085-phase05-backend",
        "backend.kobo",
        r#"
#[kobo::scenario(name = "retry_worker")]
async fn retry_worker() {
    let queue = JobQueue::new();
    let tx = queue.begin();
    tokio::spawn(async move {
        tx.commit();
    });
}
"#,
    );
    let simple = Fixture::new(
        "v085-phase05-simple",
        "simple.kobo",
        r#"
fn main() {
    println!("hello");
}
"#,
    );

    let backend_out = run_kobo(&args_with_file(&["sim", "scout", "--json"], &backend.file));
    let simple_out = run_kobo(&args_with_file(&["sim", "scout", "--json"], &simple.file));
    assert_success(&backend_out, "phase 05 scout backend fixture");
    assert_success(&simple_out, "phase 05 scout simple fixture");
    assert_contains_all(
        &backend_out.combined(),
        &["top_target", "retry_worker", "reasons", "next_command"],
        "phase 05 rich fixture must produce an actionable ranked target",
    );
    assert_contains_all(
        &simple_out.combined(),
        &["no high-value simulation target", "reasons"],
        "phase 05 simple fixture must not receive a fake high-value target",
    );
    assert_ne!(
        first_named_line(&backend_out.combined(), "\"top_target\""),
        first_named_line(&simple_out.combined(), "\"top_target\""),
        "phase 05 scout ranking must change with input signals"
    );
}

#[test]
fn phase_06_nondeterminism_scanner_counts_raw_sources_and_safe_variant() {
    let raw = Fixture::new(
        "v085-phase06-raw",
        "raw.kobo",
        r#"
#[kobo::scenario(name = "raw_sources")]
fn raw_sources() {
    let now = std::time::SystemTime::now();
    let id = rand::random::<u64>();
    let file = std::fs::read_to_string("input.txt").unwrap();
    tokio::spawn(async move {
        println!("{:?} {} {}", now, id, file);
    });
}
"#,
    );
    let wrapped = Fixture::new(
        "v085-phase06-wrapped",
        "wrapped.kobo",
        r#"
#[kobo::scenario(name = "wrapped_sources")]
fn wrapped_sources(clock: Clock, rng: Rng, files: Files) {
    let now = clock.now();
    let id = rng.next_u64();
    let file = files.read("input.txt");
    println!("{:?} {} {}", now, id, file);
}
"#,
    );

    let raw_out = run_kobo(&args_with_file(
        &["check", "--error-format=json"],
        &raw.file,
    ));
    let wrapped_out = run_kobo(&args_with_file(
        &["check", "--error-format=json"],
        &wrapped.file,
    ));
    let raw_codes = diagnostic_codes(&raw_out);
    let wrapped_codes = diagnostic_codes(&wrapped_out);

    assert!(
        count_code(&raw_codes, "K0102") >= 4,
        "phase 06 raw scenario must report each nondeterminism class as K0102, got {raw_codes:?}\n{}",
        raw_out.combined()
    );
    assert_eq!(
        count_code(&wrapped_codes, "K0102"),
        0,
        "phase 06 wrapper variant must remove raw nondeterminism diagnostics:\n{}",
        wrapped_out.combined()
    );
}

#[test]
fn phase_07_kwit_replay_validates_schema_without_claiming_full_replay() {
    let valid = Fixture::new(
        "v085-phase07-valid",
        "valid.kwit",
        r#"{
  "schema_version": 0,
  "source": {"path": "src/main.kobo", "span": {"start": 1, "end": 12}},
  "scenario": "retry_worker",
  "events": [{"kind": "spawn", "span": {"start": 4, "end": 9}}],
  "unknown_future_field": {"kept": true}
}
"#,
    );
    let invalid = Fixture::new(
        "v085-phase07-invalid",
        "invalid.kwit",
        r#"{
  "schema_version": 999,
  "scenario": "retry_worker",
  "events": []
}
"#,
    );

    let valid_out = run_kobo(&args_with_file(&["replay"], &valid.file));
    let invalid_out = run_kobo(&args_with_file(&["replay"], &invalid.file));
    assert_success(&valid_out, "phase 07 replay valid witness");
    assert_failure(&invalid_out, "phase 07 replay invalid witness");
    assert_contains_all(
        &valid_out.combined(),
        &["metadata-only", "schema_version: 0", "src/main.kobo"],
        "phase 07 valid witness should print only schema/source metadata",
    );
    assert_not_contains(
        &valid_out.combined().to_lowercase(),
        "replayed successfully",
        "phase 07 must not claim deterministic replay completion",
    );
    assert_contains_all(
        &invalid_out.combined(),
        &["schema_version", "unsupported", "K"],
        "phase 07 invalid witness must fail with registry-backed diagnostic wording",
    );
}

#[test]
fn phase_08_scenario_metadata_is_indexed_without_hidden_simulation() {
    let scenario = Fixture::new(
        "v085-phase08-scenario",
        "scenario.kobo",
        r#"
#[kobo::scenario(name = "payment_retry", tags = ["io", "retry"])]
fn payment_retry() {
    println!("retry");
}
"#,
    );
    let ordinary = Fixture::new(
        "v085-phase08-ordinary",
        "ordinary.kobo",
        r#"
fn payment_retry() {
    println!("retry");
}
"#,
    );

    let scenario_out = run_kobo(&args_with_file(
        &["inspect", "--scenario-metadata"],
        &scenario.file,
    ));
    let ordinary_out = run_kobo(&args_with_file(
        &["inspect", "--scenario-metadata"],
        &ordinary.file,
    ));
    assert_success(&scenario_out, "phase 08 scenario inspect");
    assert_success(&ordinary_out, "phase 08 ordinary inspect");
    assert_contains_all(
        &scenario_out.combined(),
        &["scenario", "payment_retry", "io", "retry"],
        "phase 08 scenario metadata must be visible through inspect",
    );
    assert_not_contains(
        &ordinary_out.combined(),
        "scenario",
        "phase 08 ordinary functions must not receive fake scenario metadata",
    );
    assert_not_contains(
        &scenario_out.combined().to_lowercase(),
        "simulated time",
        "phase 08 scenario attributes must not silently enable simulation",
    );
}

#[test]
fn phase_09_boundary_policy_keeps_normal_crates_compatible_and_replay_explicit() {
    let normal = Fixture::new(
        "v085-phase09-normal",
        "normal.kobo",
        r#"
use reqwest::Client;

fn main() {
    let client = Client::new();
    println!("{:?}", client);
}
"#,
    );
    let replay_critical = Fixture::new(
        "v085-phase09-replay",
        "replay.kobo",
        r#"
use reqwest::Client;

#[kobo::scenario(name = "fetch_user")]
fn fetch_user() {
    let client = Client::new();
    println!("{:?}", client);
}
"#,
    );
    let suppressed = Fixture::new(
        "v085-phase09-suppressed",
        "suppressed.kobo",
        r#"
#[kobo::boundary(crate = "reqwest", policy = "outside", reason = "integration tested by service contract")]
use reqwest::Client;

#[kobo::scenario(name = "fetch_user")]
fn fetch_user() {
    let client = Client::new();
    println!("{:?}", client);
}
"#,
    );

    let normal_out = run_kobo(&args_with_file(
        &["check", "--error-format=json"],
        &normal.file,
    ));
    let replay_out = run_kobo(&args_with_file(
        &["check", "--replay-critical", "--error-format=json"],
        &replay_critical.file,
    ));
    let suppressed_out = run_kobo(&args_with_file(
        &["check", "--replay-critical", "--error-format=json"],
        &suppressed.file,
    ));

    assert_not_contains(
        &normal_out.combined(),
        "K0107",
        "phase 09 normal external crate use must not be blocked by replay policy",
    );
    assert_contains_all(
        &replay_out.combined(),
        &["K0107", "model", "record", "outside", "opaque", "debt"],
        "phase 09 replay-critical external boundary must emit explicit policy vocabulary",
    );
    assert_contains_all(
        &suppressed_out.combined(),
        &["K0108", "integration tested by service contract"],
        "phase 09 reason-bearing boundary suppression must be recorded",
    );
}

#[test]
fn phase_10_backend_registry_recommendations_are_evidence_specific_and_metadata_only() {
    let async_boundary = Fixture::new(
        "v085-phase10-async",
        "async_boundary.kobo",
        r#"
#[kobo::scenario(name = "worker_race")]
async fn worker_race() {
    tokio::spawn(async move {
        println!("work");
    });
}
"#,
    );
    let property_boundary = Fixture::new(
        "v085-phase10-property",
        "property_boundary.kobo",
        r#"
#[kobo::scenario(name = "parser_property")]
fn parser_property(input: String) {
    let parsed = parse(input);
    assert(parsed.is_ok());
}
"#,
    );

    let async_out = run_kobo(&args_with_file(
        &["sim", "scout", "--backend-recommendations"],
        &async_boundary.file,
    ));
    let property_out = run_kobo(&args_with_file(
        &["sim", "scout", "--backend-recommendations"],
        &property_boundary.file,
    ));
    assert_success(&async_out, "phase 10 async backend recommendation");
    assert_success(&property_out, "phase 10 property backend recommendation");
    assert_contains_all(
        &async_out.combined(),
        &["Loom", "Shuttle", "backend_fit"],
        "phase 10 async boundary should recommend concurrency DST backends",
    );
    assert_contains_all(
        &property_out.combined(),
        &["proptest", "backend_fit"],
        "phase 10 property fixture should recommend property testing backend",
    );
    assert_ne!(
        async_out.combined(),
        property_out.combined(),
        "phase 10 backend registry cannot return one fixed recommendation",
    );
    assert_not_contains(
        &async_out.combined(),
        "executing backend",
        "phase 10 registry must not execute DST backends in v0.8.5",
    );
}

#[test]
fn phase_10_typescript_posture_controls_are_kobo_first_and_reserved() {
    let service = Fixture::new(
        "v085-phase10-posture",
        "service.kobo",
        r#"
use axum::Router;
use reqwest::Client;

#[kobo::scenario(name = "service_retry")]
async fn service_retry() {
    let client = Client::new();
    tokio::spawn(async move {
        println!("{:?}", client);
    });
}
"#,
    );

    let why = run_kobo(&args_with_file(
        &["sim", "scout", "--why", "--json"],
        &service.file,
    ));
    assert_success(&why, "v0.8.5 posture scout --why");
    assert_contains_all(
        &why.combined(),
        &[
            "Kobo is Rust-shaped and Cargo-native",
            "possible engines",
            "backend_fit",
            "Loom",
            "Shuttle",
            "normal Kobo source stays framework-shaped",
        ],
        "scout --why must explain backend fit through Kobo concepts",
    );
    assert_not_contains(
        &why.combined(),
        "shuttle::sync",
        "scout --why must not tell users to import backend replacement types",
    );
    assert_not_contains(
        &why.combined(),
        "loom::sync",
        "scout --why must not tell users to import backend replacement types",
    );

    let sim = run_kobo(&args_with_file(&["inspect", "--sim"], &service.file));
    assert_success(&sim, "v0.8.5 posture inspect --sim");
    assert_contains_all(
        &sim.combined(),
        &[
            "inspect --sim",
            "simulation contract transparency",
            "Rust-shaped",
            "Cargo-native",
            "use --harness",
        ],
        "inspect --sim must expose the public transparency path without backend leakage",
    );
    assert_not_contains(
        &sim.combined(),
        "use loom::",
        "inspect --sim must not emit backend-native imports",
    );
    assert_not_contains(
        &sim.combined(),
        "use shuttle::",
        "inspect --sim must not emit backend-native imports",
    );

    let harness = run_kobo(&args_with_file(
        &["inspect", "--sim", "--harness"],
        &service.file,
    ));
    assert_success(&harness, "v0.8.5 posture inspect --sim --harness");
    assert_contains_all(
        &harness.combined(),
        &[
            "inspect --sim --harness",
            "backend adapter boundary",
            "user source remains normal",
        ],
        "inspect --sim --harness must expose backend adapter boundaries honestly",
    );
}

#[test]
fn phase_11_contextual_suggestions_are_pattern_specific_and_capped() {
    let async_send = Fixture::new(
        "v085-phase11-async",
        "async_send.kobo",
        r#"
async fn worker() {
    let mut shared = Vec::new();
    tokio::spawn(async move {
        shared.push(1);
    });
}
"#,
    );
    let must_call = Fixture::new(
        "v085-phase11-liveness",
        "must_call.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction { id: u64 }

fn handler(fail: bool) {
    let tx = Transaction { id: 1 };
    if fail {
        return;
    }
    tx.commit();
}
"#,
    );
    let quiet = Fixture::new(
        "v085-phase11-quiet",
        "quiet.kobo",
        "fn main() { println!(\"hello\"); }\n",
    );

    let async_out = run_kobo(&args_with_file(
        &["check", "--checked", "--error-format=json"],
        &async_send.file,
    ));
    let liveness_out = run_kobo(&args_with_file(
        &["check", "--checked", "--error-format=json"],
        &must_call.file,
    ));
    let quiet_out = run_kobo(&args_with_file(
        &["check", "--checked", "--error-format=json"],
        &quiet.file,
    ));
    assert_contains_all(
        &async_out.combined(),
        &["suggestions", "async_shared", "LocalSet", "confidence"],
        "phase 11 async pattern needs async-specific suggestions",
    );
    assert_contains_all(
        &liveness_out.combined(),
        &["suggestions", "debt --liveness", "confidence"],
        "phase 11 repeated obligation pattern needs liveness suggestion",
    );
    assert_not_contains(
        &quiet_out.combined(),
        "suggestions",
        "phase 11 quiet fixture must not receive generic suggestions",
    );
    assert_ne!(
        async_out.combined(),
        liveness_out.combined(),
        "phase 11 suggestions must depend on the source pattern",
    );
}

#[test]
fn phase_12_warning_budget_prioritizes_visible_region_without_losing_machine_output() {
    let many = Fixture::new(
        "v085-phase12-budget",
        "budget.kobo",
        r#"
fn first() {
    let a = String::from("a");
    consume(a);
    println!("{}", a);
}

fn visible() {
    let b = String::from("b");
    consume(b);
    println!("{}", b);
}

fn last() {
    let c = String::from("c");
    consume(c);
    println!("{}", c);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );

    let visible_out = run_kobo(&args_with_file(
        &[
            "check",
            "--checked",
            "--error-format=json",
            "--max-diagnostics=1",
            "--visible-region=8:1-12:1",
        ],
        &many.file,
    ));
    let top_out = run_kobo(&args_with_file(
        &[
            "check",
            "--checked",
            "--error-format=json",
            "--max-diagnostics=1",
            "--visible-region=1:1-5:1",
        ],
        &many.file,
    ));
    assert_contains_all(
        &visible_out.combined(),
        &["visible", "budgeted", "grouped", "machine_diagnostics"],
        "phase 12 visible region should survive the human output cap",
    );
    assert_contains_all(
        &top_out.combined(),
        &["first", "budgeted", "grouped", "machine_diagnostics"],
        "phase 12 changing visible region should change prioritized output",
    );
    assert_ne!(
        visible_out.combined(),
        top_out.combined(),
        "phase 12 warning budget cannot ignore visible-region input"
    );
}

#[test]
fn phase_13_k0061_rich_card_has_actionable_no_color_paths_without_internal_types() {
    let fixture = Fixture::new(
        "v085-phase13-k0061",
        "k0061.kobo",
        r#"
async fn main() {
    let mut state = Vec::new();
    let borrowed = &mut state;
    tokio::spawn(async move {
        borrowed.push(1);
    });
    state.push(2);
}
"#,
    );

    let output = run_kobo_in(&args_with_file(&["check", "--strict"], &fixture.file), None);
    let text = output.combined();
    assert_contains_all(
        &text,
        &["K0061", "LocalSet", "async_shared", "narrow guard", "-->"],
        "phase 13 K0061 card must show explicit source-mapped fix paths",
    );
    assert_not_contains(
        &text,
        "Owned<",
        "phase 13 diagnostics must not leak compiler-internal owned wrappers",
    );
    assert_not_contains(
        &text,
        "\u{1b}[",
        "phase 13 NO_COLOR fallback should be plain text",
    );
}

#[test]
fn phase_14_kobo_fix_applies_only_machine_applicable_idempotent_edits() {
    let fixture = Fixture::new(
        "v085-phase14-fix",
        "fixable.kobo",
        r#"
fn main() {
    let name = String::from("alpha");
    consume(name);
    println!("{}", name);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );
    let original = fixture.read_source();
    let dry_run = run_kobo(&args_with_file(
        &["fix", "--dry-run", "--json"],
        &fixture.file,
    ));
    assert_success(&dry_run, "phase 14 fix dry-run");
    assert_contains_all(
        &dry_run.combined(),
        &["MachineApplicable", "TextEdit", "clone"],
        "phase 14 dry-run must expose only safe machine-applicable edits",
    );
    assert_eq!(
        fixture.read_source(),
        original,
        "phase 14 dry-run must not modify the fixture"
    );

    let apply = run_kobo(&args_with_file(&["fix", "--apply"], &fixture.file));
    assert_success(&apply, "phase 14 fix apply");
    let after_apply = fixture.read_source();
    assert_ne!(
        after_apply, original,
        "phase 14 apply must change the fixable source"
    );
    let second_apply = run_kobo(&args_with_file(&["fix", "--apply"], &fixture.file));
    assert_success(&second_apply, "phase 14 second fix apply");
    assert_eq!(
        fixture.read_source(),
        after_apply,
        "phase 14 apply must be idempotent after the first safe edit"
    );
}

#[test]
fn phase_15_field_capability_views_lower_to_real_field_borrows() {
    let header_only = Fixture::new(
        "v085-phase15-header",
        "header.kobo",
        r#"
struct Request {
    header: String,
    body: String,
}

fn route(req: Request using { header }) {
    println!("{}", req.header);
}
"#,
    );
    let header_and_body = Fixture::new(
        "v085-phase15-header-body",
        "header_body.kobo",
        r#"
struct Request {
    header: String,
    body: String,
}

fn route(req: Request using { header, body }) {
    println!("{} {}", req.header, req.body);
}
"#,
    );
    let duplicate = Fixture::new(
        "v085-phase15-duplicate",
        "duplicate.kobo",
        r#"
struct Request {
    header: String,
    body: String,
}

fn route(req: Request using { header, header }) {
    println!("{}", req.header);
}
"#,
    );

    let header_out = run_kobo(&args_with_file(&["inspect", "--clean"], &header_only.file));
    let both_out = run_kobo(&args_with_file(
        &["inspect", "--clean"],
        &header_and_body.file,
    ));
    let duplicate_out = run_kobo(&args_with_file(
        &["check", "--error-format=json"],
        &duplicate.file,
    ));
    assert_success(&header_out, "phase 15 header-only view inspect");
    assert_success(&both_out, "phase 15 header-body view inspect");
    assert_contains_all(
        &header_out.combined(),
        &["&req.header", "using"],
        "phase 15 generated Rust must reflect the field capability list",
    );
    assert_contains_all(
        &both_out.combined(),
        &["&req.header", "&req.body"],
        "phase 15 changing field list must change lowered Rust",
    );
    assert_ne!(
        header_out.combined(),
        both_out.combined(),
        "phase 15 lowering cannot ignore the using field list"
    );
    assert_contains_all(
        &duplicate_out.combined(),
        &["K", "duplicate", "header"],
        "phase 15 duplicate field capability must be a registry diagnostic",
    );
}

#[test]
fn phase_16_doctor_profiles_and_trait_defaults_are_project_sensitive() {
    let api = Fixture::project(
        "v085-phase16-api",
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "api_fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
reqwest = "0.12"
"#,
            ),
            (
                "src/main.kobo",
                r#"
trait Store {
    fn get(&self, key: String) -> String;
}

fn handler(store: dyn Store) {
    println!("{}", store.get(String::from("user")));
}
"#,
            ),
        ],
    );
    let game = Fixture::project(
        "v085-phase16-game",
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "game_fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = "0.13"
"#,
            ),
            (
                "src/main.kobo",
                r#"
trait System {
    fn tick(&self);
}

#[kobo::generic]
fn run_system<T: System>(system: T) {
    system.tick();
}
"#,
            ),
        ],
    );

    let api_doctor = run_kobo_in(&args(&["doctor", "--deps", "--json"]), Some(&api.root));
    let game_doctor = run_kobo_in(&args(&["doctor", "--deps", "--json"]), Some(&game.root));
    assert_success(&api_doctor, "phase 16 api doctor");
    assert_success(&game_doctor, "phase 16 game doctor");
    assert_contains_all(
        &api_doctor.combined(),
        &["api-service", "proc_macro", "feature", "msrv"],
        "phase 16 doctor must emit API-service dependency hints",
    );
    assert_contains_all(
        &game_doctor.combined(),
        &["game-server-core", "feature", "msrv"],
        "phase 16 doctor must emit game-server dependency hints",
    );
    assert_ne!(
        api_doctor.combined(),
        game_doctor.combined(),
        "phase 16 doctor cannot print one fixed project report"
    );

    let api_inspect = run_kobo(&args_with_file(
        &[
            "inspect",
            "--profile=api-service",
            "--trait-default=trait-object-first",
        ],
        &api.file,
    ));
    let game_inspect = run_kobo(&args_with_file(
        &[
            "inspect",
            "--profile=game-server-core",
            "--trait-default=generic",
        ],
        &game.file,
    ));
    assert_success(&api_inspect, "phase 16 api trait default inspect");
    assert_success(&game_inspect, "phase 16 game generic inspect");
    assert_contains_all(
        &api_inspect.combined(),
        &["Box<dyn Store>", "thin facade"],
        "phase 16 script/checked default should prefer trait-object facade",
    );
    assert_contains_all(
        &game_inspect.combined(),
        &["fn run_system<T: System>", "generic facade"],
        "phase 16 generic annotation/profile should opt into monomorphization",
    );
}
