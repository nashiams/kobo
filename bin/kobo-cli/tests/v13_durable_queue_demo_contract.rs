mod v09_common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V13_TIMEOUT: Duration = Duration::from_secs(90);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn durable_queue_crash_source() -> &'static str {
    r#"
struct DurableQueue {}

// kobo:temporal never storage-crash-after-write
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn crash_after_ack() {
    let _queue = DurableQueue {};
    let delivery = Delivery {};
    delivery.ack();
    ward.storage.write("pending");
    ward.storage.crash_after_write();
}
"#
}

fn durable_queue_inferred_source() -> &'static str {
    r#"
struct DurableQueue {}
struct StorageFacade {}
struct Queue {}
struct Parcel {}

impl Queue {
    async fn recv(&mut self) -> Parcel { Parcel {} }
}

impl Parcel {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

impl StorageFacade {
    fn append(&mut self, _record: &str) {}
    fn flush(&mut self) {}
}

#[kobo::scenario(profile = "async")]
async fn durable_queue_ok() {
    let mut queue = Queue {};
    let mut storage = StorageFacade {};
    let delivery = queue.recv().await;
    storage.append("pending");
    storage.flush();
    delivery.ack();
}
"#
}

fn durable_queue_clean_source() -> String {
    format!("{}\nfn main() {{}}\n", durable_queue_inferred_source())
}

fn durable_queue_metadata_source() -> &'static str {
    r#"
ward DurableQueue {
    state log: Vec<String>
    state pending: Vec<String>

    obligation Delivery must ack | nack | requeue
    port storage: durable_log
    recording ack_log
    debt external_metrics

    scenario metadata_case {
        let delivery = Delivery {};
        delivery.ack();
    }
}
"#
}

fn run_demo_test(
    project: &TestProject,
    source: &str,
    target: &str,
    expect_success: bool,
) -> (CliOutput, PathBuf, Value) {
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--engine"),
            s("both"),
            s("--seed"),
            s("1317"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s(target),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    if expect_success {
        assert_success(&output, "durable queue demo scenario");
    } else {
        assert_failure(&output, "durable queue demo scenario");
    }
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("durable queue demo should emit a witness");
    let witness =
        serde_json::from_str(&std::fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    (output, witness_path, witness)
}

#[test]
fn durable_queue_finds_or_proves_crash_after_ack() {
    let project = TestProject::new("v13-durable-queue-crash");
    let (output, _witness_path, witness) = run_demo_test(
        &project,
        durable_queue_crash_source(),
        "crash_after_ack",
        false,
    );

    assert_contains(
        &output.combined(),
        "lost-message",
        "demo should find the crash-after-ack lost-message history",
    );
    assert_eq!(witness["flagship_demo"]["name"], "durable_queue");
    assert_eq!(witness["failure"]["mode"], "lost-message");
    assert_eq!(witness["temporal_checks"][0]["status"], "failed");
    assert_contains(
        &witness["events"].to_string(),
        "storage-crash-after-write",
        "witness should carry the crash history event",
    );
    assert_eq!(
        witness["flagship_demo"]["evidence_inputs"]["source"], "compiler-scenario-program",
        "demo evidence should be derived from compiler scenario facts"
    );
    assert_contains(
        &witness["flagship_demo"]["evidence_inputs"]["storage_actions"].to_string(),
        "crash_after_write",
        "demo evidence should expose concrete storage actions",
    );
}

#[test]
fn durable_queue_emits_replayable_kwit_witness() {
    let project = TestProject::new("v13-durable-queue-replay");
    let (_output, witness_path, witness) = run_demo_test(
        &project,
        durable_queue_crash_source(),
        "crash_after_ack",
        false,
    );
    assert_eq!(witness["replay_guarantee"], "exact");
    assert_eq!(
        witness["flagship_demo"]["replayable_kwit"],
        Value::Bool(true)
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_success(&replay, "durable queue demo witness should replay");
    assert_contains(
        &replay.combined(),
        r#""replay":"exact""#,
        "demo replay should claim exact only after witness validation",
    );
}

#[test]
fn durable_queue_ack_nack_requeue_inferred_without_manual_declarations() {
    let project = TestProject::new("v13-durable-queue-inferred");
    let (_output, _witness_path, witness) = run_demo_test(
        &project,
        durable_queue_inferred_source(),
        "durable_queue_ok",
        true,
    );

    let queue_delivery = witness["inferred_obligations"]
        .as_array()
        .expect("inferred obligations should be present")
        .iter()
        .find(|entry| entry["template_id"].as_str() == Some("queue_delivery"))
        .unwrap_or_else(|| {
            panic!(
                "missing queue_delivery in {}",
                witness["inferred_obligations"]
            )
        });
    assert_eq!(queue_delivery["state"], "discharged");
    for action in ["ack", "nack", "requeue"] {
        assert_contains(
            &queue_delivery["terminal_actions"].to_string(),
            action,
            "queue delivery lifecycle should be inferred without manual declaration",
        );
    }
}

#[test]
fn durable_queue_clean_rust_output_builds() {
    let project = TestProject::new("v13-durable-queue-clean-rust");
    let clean_source = durable_queue_clean_source();
    let file = project.main_file(&clean_source);
    let out_dir = project.root.join("target/durable-clean");
    let output = run_kobo(
        &[
            s("inspect"),
            s("--clean"),
            s("--cargo"),
            path_arg(&out_dir),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "durable queue clean Rust cargo export");

    let cargo = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(out_dir.join("Cargo.toml"))
        .output()
        .expect("cargo check should launch for clean Rust output");
    let cargo_output = CliOutput {
        status: cargo.status,
        stdout: String::from_utf8_lossy(&cargo.stdout).to_string(),
        stderr: String::from_utf8_lossy(&cargo.stderr).to_string(),
    };
    assert_success(
        &cargo_output,
        "durable queue clean Rust output should build",
    );
}

#[test]
fn durable_queue_ports_recordings_and_debt_are_visible() {
    let project = TestProject::new("v13-durable-queue-metadata");
    let file = project.main_file(durable_queue_metadata_source());
    let inspect = run_kobo(
        &[s("inspect"), s("--scenario-metadata"), path_arg(&file)],
        &project.root,
    );
    assert_success(&inspect, "durable queue metadata inspect");
    let text = inspect.combined();
    for expected in [
        "ward DurableQueue",
        "port storage",
        "recording ack_log",
        "debt external_metrics",
    ] {
        assert_contains(
            &text,
            expected,
            "durable queue external facts should be visible",
        );
    }
}
