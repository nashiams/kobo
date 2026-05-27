//! Edge-case RED oracle tests for the plan.
//!
//! `foundation_oracles.rs` provides one broad public-surface test per slice.
//! This file adds adversarial and boundary variants so an implementation cannot
//! pass by printing one fixed answer or by handling only the happy-path fixture.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    file: PathBuf,
}

impl Fixture {
    fn file(label: &str, file_name: &str, source: &str) -> Self {
        let root = unique_root(label);
        fs::create_dir_all(&root).expect("fixture root should be creatable");
        let file = root.join(file_name);
        fs::write(&file, source).expect("fixture source should be writable");
        Self { root, file }
    }

    fn project(label: &str, files: &[(&str, &str)]) -> Self {
        let root = unique_root(label);
        for (relative, contents) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("fixture parent should be creatable");
            }
            fs::write(path, contents).expect("project fixture file should be writable");
        }
        Self {
            file: root.join("src/main.kobo"),
            root,
        }
    }

    fn read(&self) -> String {
        fs::read_to_string(&self.file).expect("fixture should read")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct OutputText {
    success: bool,
    stdout: String,
    stderr: String,
}

impl OutputText {
    fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn unique_root(label: &str) -> PathBuf {
    let id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}-{}-{id}", std::process::id()))
}

fn run(args: &[String]) -> OutputText {
    run_in(args, None)
}

fn run_in(args: &[String], cwd: Option<&Path>) -> OutputText {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kobo"));
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().expect("kobo command should launch");
    OutputText {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn with_file(values: &[&str], file: &Path) -> Vec<String> {
    let mut args = strings(values);
    args.push(file.to_string_lossy().into_owned());
    args
}

fn assert_success(output: &OutputText, context: &str) {
    assert!(
        output.success,
        "{context} must succeed\nstdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
}

fn assert_failure(output: &OutputText, context: &str) {
    assert!(
        !output.success,
        "{context} must fail\nstdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
}

fn assert_has(text: &str, needle: &str, context: &str) {
    assert!(
        text.contains(needle),
        "{context}\nmissing `{needle}` in:\n{text}"
    );
}

fn assert_lacks(text: &str, needle: &str, context: &str) {
    assert!(
        !text.contains(needle),
        "{context}\nunexpected `{needle}` in:\n{text}"
    );
}

fn json_lines(output: &OutputText) -> Vec<Value> {
    output
        .combined()
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('{') {
                serde_json::from_str(trimmed).ok()
            } else {
                None
            }
        })
        .collect()
}

fn json_codes(output: &OutputText) -> Vec<String> {
    json_lines(output)
        .into_iter()
        .filter_map(|value| value["code"].as_str().map(ToOwned::to_owned))
        .collect()
}

fn code_count(codes: &[String], code: &str) -> usize {
    codes
        .iter()
        .filter(|actual| actual.as_str() == code)
        .count()
}

#[test]
fn slice_00_edge_unknown_and_case_variant_explain_are_structured() {
    let lowercase = run(&strings(&["explain", "k0107", "--verbose"]));
    let unknown = run(&strings(&["explain", "K9999"]));

    assert_success(&lowercase, "slice 00 lowercase diagnostic explain");
    assert_failure(&unknown, "slice 00 unknown diagnostic explain");
    assert_has(
        &lowercase.combined(),
        "K0107",
        "slice 00 explain should normalize user-entered diagnostic codes",
    );
    assert_has(
        &lowercase.combined(),
        "slug: unmodeled-external-boundary",
        "slice 00 normalized explain must still show registry slug",
    );
    assert_has(
        &unknown.combined(),
        "unknown diagnostic code",
        "slice 00 unknown explain must be a structured user-facing error",
    );
    assert_has(
        &unknown.combined(),
        "nearest",
        "slice 00 unknown explain should suggest nearby registered codes",
    );
}

#[test]
fn slice_01_edge_multiple_parse_errors_keep_distinct_spans_and_good_analysis() {
    let fixture = Fixture::file(
        "foundation-edge-slice01",
        "multi_bad.kobo",
        r#"
fn first() {
    let a =
}

fn second() {
    if true {
}

fn good() {
    let name = String::from("edge");
    consume(name);
    println!("{}", name);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );

    let output = run(&with_file(
        &[
            "check",
            "--checked",
            "--recover-parse",
            "--error-format=json",
        ],
        &fixture.file,
    ));
    let values = json_lines(&output);
    let codes = json_codes(&output);
    let recovery_lines = values
        .iter()
        .filter(|value| {
            value["code"]
                .as_str()
                .is_some_and(|code| code.starts_with("K011"))
        })
        .filter_map(|value| value["primary"]["line_start"].as_u64())
        .collect::<Vec<_>>();

    assert!(
        recovery_lines.len() >= 2,
        "slice 01 must report multiple independent K011x recovery diagnostics, got {codes:?}\n{}",
        output.combined()
    );
    assert_ne!(
        recovery_lines[0], recovery_lines[1],
        "slice 01 recovery diagnostics must preserve distinct source spans"
    );
    assert!(
        codes.iter().any(|code| code == "K0001"),
        "slice 01 recovery must still analyze the non-poisoned good function, got {codes:?}"
    );
}

#[test]
fn slice_02_edge_lsp_handles_no_project_and_exposes_code_action_data() {
    let fixture = Fixture::file(
        "foundation-edge-slice02",
        "no_project.kobo",
        r#"
fn main() {
    let value = String::from("edge");
    consume(value);
    println!("{}", value);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );

    let output = run(&with_file(
        &["lsp-diagnostics", "--format=json", "--no-project-ok"],
        &fixture.file,
    ));
    assert_success(&output, "slice 02 LSP no-project diagnostic export");
    let text = output.combined();
    assert_has(
        &text,
        "\"code\":\"K0001\"",
        "slice 02 LSP must publish normal compiler diagnostics",
    );
    assert_has(
        &text,
        "\"codeDescription\"",
        "slice 02 LSP diagnostic must include explain navigation",
    );
    assert_has(
        &text,
        "\"data\"",
        "slice 02 LSP diagnostic must include Kobo-specific machine data",
    );
}

#[test]
fn slice_03_edge_malformed_must_call_is_registry_diagnostic() {
    let empty = Fixture::file(
        "foundation-edge-slice03-empty",
        "empty_must_call.kobo",
        r#"
#[kobo::must_call()]
struct Transaction {
    id: u64,
}
"#,
    );
    let malformed = Fixture::file(
        "foundation-edge-slice03-malformed",
        "malformed_must_call.kobo",
        r#"
#[kobo::must_call(commit || rollback)]
struct Transaction {
    id: u64,
}
"#,
    );

    for fixture in [&empty, &malformed] {
        let output = run(&with_file(&["check", "--error-format=json"], &fixture.file));
        assert_failure(&output, "slice 03 malformed must_call should fail");
        let text = output.combined();
        assert_has(
            &text,
            "must_call",
            "slice 03 malformed metadata diagnostic must name the attribute",
        );
        assert_has(
            &text,
            "\"slug\"",
            "slice 03 malformed metadata diagnostic must use registry identity",
        );
        assert_lacks(
            &text,
            "panicked",
            "slice 03 malformed metadata must not crash the compiler",
        );
    }
}

#[test]
fn slice_04_edge_escape_and_suppression_reason_are_not_same_diagnostic() {
    let escape = Fixture::file(
        "foundation-edge-slice04-escape",
        "escape.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction { id: u64 }

fn make() -> Transaction {
    Transaction { id: 1 }
}
"#,
    );
    let suppression_without_reason = Fixture::file(
        "foundation-edge-slice04-no-reason",
        "suppressed_no_reason.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction { id: u64 }

#[kobo::suppress(K0100)]
fn handler() {
    let tx = Transaction { id: 1 };
    println!("{}", tx.id);
}
"#,
    );
    let suppression_with_reason = Fixture::file(
        "foundation-edge-slice04-reason",
        "suppressed_reason.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction { id: u64 }

#[kobo::suppress(K0100, reason = "transaction is cleaned by outer middleware")]
fn handler() {
    let tx = Transaction { id: 1 };
    println!("{}", tx.id);
}
"#,
    );

    let escape_out = run(&with_file(&["debt", "--liveness"], &escape.file));
    let no_reason_out = run(&with_file(
        &["debt", "--liveness"],
        &suppression_without_reason.file,
    ));
    let reason_out = run(&with_file(
        &["debt", "--liveness"],
        &suppression_with_reason.file,
    ));
    assert_has(
        &escape_out.combined(),
        "K0101",
        "slice 04 escaping obligation must use K0101, not generic K0100",
    );
    assert_has(
        &no_reason_out.combined(),
        "reason",
        "slice 04 suppression without reason must be rejected",
    );
    assert_has(
        &reason_out.combined(),
        "K0108",
        "slice 04 reasoned suppression must be recorded as K0108",
    );
    assert_ne!(
        escape_out.combined(),
        reason_out.combined(),
        "slice 04 escape and suppression reports cannot be one hardcoded message"
    );
}

#[test]
fn slice_05_edge_scout_is_stable_and_does_not_modify_source() {
    let fixture = Fixture::file(
        "foundation-edge-slice05",
        "scout_stable.kobo",
        r#"
#[kobo::scenario(name = "timer_retry")]
async fn timer_retry() {
    let deadline = std::time::Instant::now();
    tokio::spawn(async move {
        println!("{:?}", deadline);
    });
}
"#,
    );

    let before = fixture.read();
    let first = run(&with_file(&["sim", "scout", "--json"], &fixture.file));
    let second = run(&with_file(&["sim", "scout", "--json"], &fixture.file));

    assert_success(&first, "slice 05 first scout run");
    assert_success(&second, "slice 05 second scout run");
    assert_eq!(
        before,
        fixture.read(),
        "slice 05 scout must not modify source files"
    );
    assert_eq!(
        first.combined(),
        second.combined(),
        "slice 05 scout output must be stable for identical input"
    );
    assert_has(
        &first.combined(),
        "next_command",
        "slice 05 scout output must include an actionable next command",
    );
}

#[test]
fn slice_06_edge_duplicate_raw_sources_are_grouped_but_counted() {
    let fixture = Fixture::file(
        "foundation-edge-slice06",
        "duplicate_raw.kobo",
        r#"
#[kobo::scenario(name = "duplicate_time")]
fn duplicate_time() {
    let start = std::time::SystemTime::now();
    let end = std::time::SystemTime::now();
    let seed = rand::random::<u64>();
    println!("{:?} {:?} {}", start, end, seed);
}
"#,
    );

    let output = run(&with_file(&["check", "--error-format=json"], &fixture.file));
    let codes = json_codes(&output);
    let text = output.combined();
    assert!(
        code_count(&codes, "K0102") >= 2,
        "slice 06 duplicate raw sources must still count each nondeterminism class, got {codes:?}\n{text}"
    );
    assert_has(
        &text,
        "occurrences",
        "slice 06 duplicate raw source groups must expose machine-readable counts",
    );
    assert_has(
        &text,
        "SystemTime::now",
        "slice 06 scanner must preserve source-specific raw operation names",
    );
}

#[test]
fn slice_07_edge_kwit_missing_span_fails_unknown_fields_survive() {
    let missing_span = Fixture::file(
        "foundation-edge-slice07-missing",
        "missing_span.kwit",
        r#"{
  "schema_version": 0,
  "source": {"path": "src/main.kobo"},
  "scenario": "edge"
}
"#,
    );
    let unknown_fields = Fixture::file(
        "foundation-edge-slice07-unknown",
        "unknown_fields.kwit",
        r#"{
  "schema_version": 0,
  "source": {"path": "src/main.kobo", "span": {"start": 0, "end": 4}},
  "scenario": "edge",
  "events": [],
  "future": {"scheduler": "kept"}
}
"#,
    );

    let missing_out = run(&with_file(&["replay"], &missing_span.file));
    let unknown_out = run(&with_file(
        &["replay", "--roundtrip-metadata"],
        &unknown_fields.file,
    ));
    assert_failure(&missing_out, "slice 07 witness missing span");
    assert_success(&unknown_out, "slice 07 witness unknown-field roundtrip");
    assert_has(
        &missing_out.combined(),
        "span",
        "slice 07 invalid witness must name the missing schema field",
    );
    assert_has(
        &unknown_out.combined(),
        "\"future\"",
        "slice 07 roundtrip must preserve forward-compatible unknown fields",
    );
}

#[test]
fn slice_08_edge_malformed_scenario_attribute_and_removed_variant() {
    let malformed = Fixture::file(
        "foundation-edge-slice08-malformed",
        "malformed_scenario.kobo",
        r#"
#[kobo::scenario()]
fn scenario_without_name() {}
"#,
    );
    let removed = Fixture::file(
        "foundation-edge-slice08-removed",
        "removed_scenario.kobo",
        "fn scenario_without_name() {}\n",
    );

    let malformed_out = run(&with_file(
        &["check", "--error-format=json"],
        &malformed.file,
    ));
    let removed_out = run(&with_file(
        &["inspect", "--scenario-metadata"],
        &removed.file,
    ));
    assert_failure(&malformed_out, "slice 08 malformed scenario attribute");
    assert_success(&removed_out, "slice 08 removed scenario metadata inspect");
    assert_has(
        &malformed_out.combined(),
        "scenario",
        "slice 08 malformed scenario diagnostic must name the attribute",
    );
    assert_lacks(
        &removed_out.combined(),
        "scenario",
        "slice 08 removing the attribute must remove scenario metadata",
    );
}

#[test]
fn slice_09_edge_boundary_policy_matrix_is_source_sensitive() {
    let modeled = Fixture::file(
        "foundation-edge-slice09-model",
        "modeled.kobo",
        r#"
#[kobo::boundary(crate = "reqwest", policy = "model", reason = "http model")]
use reqwest::Client;

#[kobo::scenario(name = "fetch")]
fn fetch() {
    let client = Client::new();
    println!("{:?}", client);
}
"#,
    );
    let debt = Fixture::file(
        "foundation-edge-slice09-debt",
        "debt.kobo",
        r#"
#[kobo::boundary(crate = "reqwest", policy = "debt", reason = "review later")]
use reqwest::Client;

#[kobo::scenario(name = "fetch")]
fn fetch() {
    let client = Client::new();
    println!("{:?}", client);
}
"#,
    );
    let missing_reason = Fixture::file(
        "foundation-edge-slice09-no-reason",
        "missing_reason.kobo",
        r#"
#[kobo::boundary(crate = "reqwest", policy = "outside")]
use reqwest::Client;
"#,
    );

    let modeled_out = run(&with_file(
        &["check", "--replay-critical", "--error-format=json"],
        &modeled.file,
    ));
    let debt_out = run(&with_file(
        &["check", "--replay-critical", "--error-format=json"],
        &debt.file,
    ));
    let missing_out = run(&with_file(
        &["check", "--replay-critical", "--error-format=json"],
        &missing_reason.file,
    ));
    assert_has(
        &modeled_out.combined(),
        "model",
        "slice 09 model policy must render explicitly",
    );
    assert_has(
        &debt_out.combined(),
        "debt",
        "slice 09 debt policy must render explicitly",
    );
    assert_ne!(
        modeled_out.combined(),
        debt_out.combined(),
        "slice 09 changing boundary policy must change output"
    );
    assert_has(
        &missing_out.combined(),
        "reason",
        "slice 09 boundary policy without reason must be rejected",
    );
}

#[test]
fn slice_10_edge_backend_registry_lists_backends_without_executing_them() {
    let output = run(&strings(&["sim", "backends", "--json"]));
    assert_success(&output, "backend registry listing");
    let text = output.combined();
    let value: Value = serde_json::from_str(&output.stdout).expect("backend JSON should parse");
    assert_eq!(value["reserved_metadata_registry"]["executed"], false);
    assert_eq!(value["reserved_metadata_registry"]["metadata_only"], true);
    for backend in [
        "Loom",
        "Shuttle",
        "Turmoil",
        "Madsim",
        "proptest",
        "failpoints",
    ] {
        assert_has(
            &text,
            backend,
            "backend registry must list every reserved metadata backend",
        );
    }
    assert_lacks(
        &text,
        "executing",
        "slice 10 backend registry listing must be metadata-only",
    );
}

#[test]
fn slice_11_edge_suggestions_are_capped_and_absent_for_quiet_code() {
    let noisy = Fixture::file(
        "foundation-edge-slice11-noisy",
        "noisy.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction { id: u64 }

async fn noisy() {
    let tx = Transaction { id: 1 };
    let mut shared = Vec::new();
    tokio::spawn(async move {
        shared.push(1);
    });
    println!("{}", tx.id);
}
"#,
    );
    let quiet = Fixture::file(
        "foundation-edge-slice11-quiet",
        "quiet.kobo",
        "fn main() { println!(\"quiet\"); }\n",
    );

    let noisy_out = run(&with_file(
        &["check", "--checked", "--error-format=json"],
        &noisy.file,
    ));
    let quiet_out = run(&with_file(
        &["check", "--checked", "--error-format=json"],
        &quiet.file,
    ));
    let noisy_text = noisy_out.combined();
    let suggestion_count = noisy_text.matches("\"suggestion\"").count()
        + noisy_text.matches("\"suggestions\"").count();
    assert_has(
        &noisy_text,
        "confidence",
        "slice 11 noisy fixture must label suggestion confidence",
    );
    assert!(
        suggestion_count <= 3,
        "slice 11 suggestions must be capped, got {suggestion_count} suggestions in:\n{noisy_text}"
    );
    assert_lacks(
        &quiet_out.combined(),
        "suggestions",
        "slice 11 quiet code must not receive generic suggestions",
    );
}

#[test]
fn slice_12_edge_budgeted_human_output_keeps_full_machine_diagnostics() {
    let fixture = Fixture::file(
        "foundation-edge-slice12",
        "budget_machine.kobo",
        r#"
fn a() {
    let value = String::from("a");
    consume(value);
    println!("{}", value);
}

fn b() {
    let value = String::from("b");
    consume(value);
    println!("{}", value);
}

fn c() {
    let value = String::from("c");
    consume(value);
    println!("{}", value);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );

    let human = run(&with_file(
        &["check", "--checked", "--max-diagnostics=1"],
        &fixture.file,
    ));
    let machine = run(&with_file(
        &[
            "check",
            "--checked",
            "--error-format=json",
            "--include-budgeted",
        ],
        &fixture.file,
    ));
    assert_has(
        &human.combined(),
        "grouped",
        "slice 12 human output should group offscreen/low-priority diagnostics",
    );
    let machine_codes = json_codes(&machine);
    assert!(
        code_count(&machine_codes, "K0001") >= 3,
        "slice 12 machine output must retain budgeted diagnostics, got {machine_codes:?}\n{}",
        machine.combined()
    );
}

#[test]
fn slice_13_edge_k0061_card_is_short_but_explain_is_detailed() {
    let fixture = Fixture::file(
        "foundation-edge-slice13",
        "k0061_edge.kobo",
        r#"
async fn main() {
    let mut shared = Vec::new();
    let guard = &mut shared;
    tokio::spawn(async move {
        guard.push(1);
    });
}
"#,
    );

    let card = run(&with_file(&["check", "--checked"], &fixture.file));
    let explain = run(&strings(&["explain", "K0061"]));
    assert_has(
        &card.combined(),
        "K0061",
        "slice 13 fixture must render the rich async shared-state card",
    );
    assert_has(
        &card.combined(),
        "kobo explain K0061",
        "slice 13 short card must point to detailed explain text",
    );
    assert_has(
        &explain.combined(),
        "LocalSet",
        "slice 13 explain must include full detailed remediation paths",
    );
    assert!(
        card.combined().lines().count() < explain.combined().lines().count(),
        "slice 13 CLI card should stay shorter than detailed explain output"
    );
}

#[test]
fn slice_14_edge_fix_refuses_placeholder_and_overlapping_edits() {
    let fixture = Fixture::file(
        "foundation-edge-slice14",
        "overlap.kobo",
        r#"
fn main() {
    let value = String::from("edge");
    consume(value);
    consume(value);
    println!("{}", value);
}

fn consume(value: String) {
    println!("{}", value);
}
"#,
    );

    let before = fixture.read();
    let dry_run = run(&with_file(&["fix", "--dry-run", "--json"], &fixture.file));
    let apply = run(&with_file(&["fix", "--apply"], &fixture.file));
    assert_success(&dry_run, "slice 14 overlapping fix dry-run");
    assert_failure(&apply, "slice 14 overlapping fix apply");
    assert_has(
        &dry_run.combined(),
        "overlap",
        "slice 14 dry-run must explain overlapping edit refusal",
    );
    assert_has(
        &dry_run.combined(),
        "HasPlaceholders",
        "slice 14 dry-run must expose placeholder edits as non-applyable",
    );
    assert_eq!(
        before,
        fixture.read(),
        "slice 14 refused fixes must not modify the source"
    );
}

#[test]
fn slice_15_edge_invalid_field_capability_has_source_mapped_diagnostic() {
    let fixture = Fixture::file(
        "foundation-edge-slice15",
        "invalid_field.kobo",
        r#"
struct Request {
    header: String,
    body: String,
}

fn route(req: Request using { missing }) {
    println!("{}", req.header);
}
"#,
    );

    let output = run(&with_file(&["check", "--error-format=json"], &fixture.file));
    assert_failure(&output, "slice 15 invalid field capability");
    let text = output.combined();
    assert_has(
        &text,
        "missing",
        "slice 15 invalid field diagnostic must name the bad field",
    );
    assert_has(
        &text,
        "\"slug\"",
        "slice 15 invalid field diagnostic must be registry-backed",
    );
    assert_lacks(
        &text,
        "fn route(",
        "slice 15 invalid using syntax must not be lowered into fake Rust",
    );
}

#[test]
fn slice_16_edge_normal_project_builds_and_profile_outputs_differ() {
    let normal = Fixture::project(
        "foundation-edge-slice16-normal",
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "normal_fixture"
version = "0.1.0"
edition = "2021"
"#,
            ),
            ("src/main.kobo", "fn main() { println!(\"normal\"); }\n"),
        ],
    );
    let api = Fixture::project(
        "foundation-edge-slice16-api",
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "api_fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = "0.12"
"#,
            ),
            ("src/main.kobo", "fn main() { println!(\"api\"); }\n"),
        ],
    );
    let storage = Fixture::project(
        "foundation-edge-slice16-storage",
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "storage_fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
rocksdb = "0.22"
"#,
            ),
            ("src/main.kobo", "fn main() { println!(\"storage\"); }\n"),
        ],
    );

    let normal_build = run_in(&strings(&["build"]), Some(&normal.root));
    let api_doctor = run_in(&strings(&["doctor", "--deps", "--json"]), Some(&api.root));
    let storage_doctor = run_in(
        &strings(&["doctor", "--deps", "--json"]),
        Some(&storage.root),
    );
    assert_success(
        &normal_build,
        "slice 16 normal crate without Kobo metadata must still build",
    );
    assert_success(&api_doctor, "slice 16 API doctor fixture");
    assert_success(&storage_doctor, "slice 16 storage doctor fixture");
    assert_has(
        &api_doctor.combined(),
        "api-service",
        "slice 16 API dependency shape must select API service profile hints",
    );
    assert_has(
        &storage_doctor.combined(),
        "storage-engine",
        "slice 16 storage dependency shape must select storage profile hints",
    );
    assert_ne!(
        api_doctor.combined(),
        storage_doctor.combined(),
        "slice 16 profile recommendations must be project-sensitive"
    );
}
