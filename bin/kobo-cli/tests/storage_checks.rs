mod cli_common;

use std::fs;

use cli_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo, s, TestProject,
};

#[test]
fn durable_queue_crash_after_ack_before_commit_records_storage_witness() {
    let project = TestProject::new("runtime-storage-durable-queue");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn durable_queue() {
    let delivery = Delivery {};
    delivery.ack();
    ward.storage.write("pending");
    ward.storage.crash_after_write();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("13"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "storage crash after ack before commit must produce a witness failure",
    );
    let text = output.combined();
    assert_contains(&text, "storage", "diagnostic should name storage facade");
    assert_contains(
        &text,
        "lost-message",
        "diagnostic should report the durable queue failure mode",
    );
    let witnesses = project.find_files_with_ext("kwit");
    assert!(!witnesses.is_empty(), "storage failure should write .kwit");
    let witness = fs::read_to_string(&witnesses[0]).expect("witness should read");
    assert_contains(&witness, "storage-crash", "witness must record crash event");
    assert_contains(
        &witness,
        "boundary_policies",
        "witness must serialize boundary policies",
    );
    let witness_json: serde_json::Value =
        serde_json::from_str(&witness).expect("witness JSON should parse");
    let harness_path = witness_json["harness_manifest"]["harness_rs_path"]
        .as_str()
        .expect("storage witness should include harness source");
    let harness_source = fs::read_to_string(harness_path).expect("harness source should read");
    assert_contains(
        &harness_source,
        "std::fs::OpenOptions",
        "storage facade must execute through the generated Rust filesystem harness",
    );
    assert_contains(
        &harness_source,
        "sync_all",
        "storage commit/flush/fsync should touch durable filesystem APIs",
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witnesses[0]),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_success(&replay, "durable queue crash witness should replay");
    assert_contains(
        &replay.combined(),
        r#""replay":"exact""#,
        "storage replay should remain exact",
    );
}
