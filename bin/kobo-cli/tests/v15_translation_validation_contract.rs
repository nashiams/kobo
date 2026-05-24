mod v09_common;
mod v15_common;

use kobo_proof::{certificate_material_hash, trace_material_hash, ProofCertificate};
use serde_json::json;
use v15_common::{
    assert_failure, assert_success, emit_artifact, path_arg, queue_loop_source, read_json,
    run_kobo, s, write_json, TestProject,
};

fn emit_queue_artifact(project: &TestProject, scenario_name: &str) -> std::path::PathBuf {
    emit_artifact(
        project,
        &queue_loop_source(scenario_name, "delivery", "queue"),
        scenario_name,
    )
}

fn panic_source(scenario_name: &str) -> String {
    format!(
        r#"
struct Queue {{}}

#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Queue {{
    fn recv(&self) -> Delivery {{ Delivery {{}} }}
}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let queue = Queue {{}};
    let delivery = queue.recv();
    delivery.ack();
    panic!("boom");
}}
"#
    )
}

fn error_exit_source(scenario_name: &str) -> String {
    format!(
        r#"
struct Queue {{}}

#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Queue {{
    fn recv(&self) -> Delivery {{ Delivery {{}} }}
}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

fn fallible() -> Result<(), ()> {{ Ok(()) }}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() -> Result<(), ()> {{
    let queue = Queue {{}};
    let delivery = queue.recv();
    delivery.ack();
    fallible()?;
    Ok(())
}}
"#
    )
}

#[test]
fn generated_rust_trace_matches_core_trace_and_validates() {
    let project = TestProject::new("v15-translation-valid");
    let artifact_path = emit_queue_artifact(&project, "translation_valid_case");
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["translation_validation"]["status"], "validated");
    assert_eq!(
        artifact["generated_rust_trace"]
            .as_array()
            .expect("generated trace should be an array")
            .len(),
        artifact["core_obligation_trace"]
            .as_array()
            .expect("Core trace should be an array")
            .len()
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&output, "valid generated trace should verify");
}

#[test]
fn codegen_source_map_emits_structured_lowering_trace_metadata() {
    let project = TestProject::new("v15-translation-codegen-trace");
    let artifact_path = emit_queue_artifact(&project, "translation_codegen_trace_case");
    let artifact = read_json(&artifact_path);
    let map_path = project
        .find_files_with_ext("map")
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "main.kobo.map"))
        .expect("proof emit should write a source map");
    let source_map = read_json(&map_path);
    let lowering_trace = source_map["lowering_trace"]
        .as_array()
        .expect("source map lowering_trace should be an array");

    assert!(
        lowering_trace.iter().any(|event| event["kind"] == "create"
            && event["binding"] == "delivery"
            && event["lowering_phase"] == "kobo-codegen"),
        "codegen-owned trace must include Delivery creation: {source_map}"
    );
    assert!(
        lowering_trace
            .iter()
            .any(|event| event["kind"] == "discharge"
                && event["binding"] == "delivery"
                && event["source_map_entry_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())),
        "codegen-owned trace must include anchored Delivery discharge: {source_map}"
    );
    assert!(
        artifact["generated_rust_trace"]
            .as_array()
            .expect("generated trace should be an array")
            .iter()
            .all(|event| event["lowering_phase"] == "kobo-codegen"),
        ".kproof generated trace must consume codegen lowering metadata: {artifact}"
    );
    assert!(
        lowering_trace
            .iter()
            .filter(|event| event["kind"] != "return")
            .all(|event| event["core_event_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())),
        "codegen-owned trace must carry Core event IDs for proof-relevant events: {source_map}"
    );
}

#[test]
fn dropped_discharge_event_fails_translation_validation() {
    let project = TestProject::new("v15-translation-dropped-discharge");
    let artifact_path = emit_queue_artifact(&project, "translation_drop_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"]
        .as_array_mut()
        .expect("generated trace should be mutable")
        .retain(|event| event["kind"] != "discharge");
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "dropped discharge should reject proof");
    assert!(
        output.combined().contains("missing generated event"),
        "failure should name missing generated event: {}",
        output.combined()
    );
}

#[test]
fn inserted_escape_event_fails_translation_validation() {
    let project = TestProject::new("v15-translation-extra-event");
    let artifact_path = emit_queue_artifact(&project, "translation_extra_case");
    let mut artifact = read_json(&artifact_path);
    let mut extra = artifact["generated_rust_trace"][0].clone();
    extra["id"] = "generated-extra-escape".into();
    extra["core_event_id"] = "core-extra-escape".into();
    extra["kind"] = "escape".into();
    extra["order"] = json!(99);
    artifact["generated_rust_trace"]
        .as_array_mut()
        .expect("generated trace should be mutable")
        .push(extra);
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "inserted generated event should reject proof");
    assert!(
        output.combined().contains("extra generated event"),
        "failure should name extra generated event: {}",
        output.combined()
    );
}

#[test]
fn meaning_changing_reorder_fails_translation_validation() {
    let project = TestProject::new("v15-translation-reorder");
    let artifact_path = emit_queue_artifact(&project, "translation_reorder_case");
    let mut artifact = read_json(&artifact_path);
    let generated = artifact["generated_rust_trace"]
        .as_array_mut()
        .expect("generated trace should be mutable");
    generated[0]["order"] = json!(1);
    generated[1]["order"] = json!(0);
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "meaning-changing trace reorder should reject proof",
    );
    assert!(
        output.combined().contains("order mismatch"),
        "failure should name order mismatch: {}",
        output.combined()
    );
}

#[test]
fn stale_source_map_anchor_fails_translation_validation() {
    let project = TestProject::new("v15-translation-stale-anchor");
    let artifact_path = emit_queue_artifact(&project, "translation_stale_anchor_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"][0]["source_map_anchor"]["status"] = "stale".into();
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "stale source-map anchor should reject proof");
    assert!(
        output.combined().contains("source-map anchor"),
        "failure should name stale source-map anchor: {}",
        output.combined()
    );
}

#[test]
fn changed_template_version_fails_translation_validation() {
    let project = TestProject::new("v15-translation-template");
    let artifact_path = emit_queue_artifact(&project, "translation_template_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"][0]["template_version"] = "stale".into();
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "template mismatch should reject proof");
    assert!(
        output.combined().contains("template mismatch"),
        "failure should name template mismatch: {}",
        output.combined()
    );
}

#[test]
fn translation_status_cannot_be_toggled_without_trace_evidence() {
    let project = TestProject::new("v15-translation-status-toggle");
    let artifact_path = emit_queue_artifact(&project, "translation_status_toggle_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"] = json!([]);
    artifact["translation_validation"]["status"] = "validated".into();
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "status toggle without trace evidence should reject proof",
    );
    assert!(
        output.combined().contains("missing generated event")
            || output
                .combined()
                .contains("translation validation status mismatch"),
        "failure should name trace evidence gap: {}",
        output.combined()
    );
}

#[test]
fn generated_trace_preserves_real_panic_terminator_events() {
    let project = TestProject::new("v15-translation-panic-event");
    let artifact_path = emit_artifact(
        &project,
        &panic_source("translation_panic_event_case"),
        "translation_panic_event_case",
    );
    let artifact = read_json(&artifact_path);

    assert!(
        artifact["core_obligation_trace"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "panic")),
        "Core trace must include real panic terminator events: {artifact}"
    );
    assert!(
        artifact["generated_rust_trace"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "panic")),
        "generated trace must preserve real panic terminator events: {artifact}"
    );
    assert_eq!(artifact["translation_validation"]["status"], "validated");
}

#[test]
fn generated_trace_preserves_normal_return_events() {
    let project = TestProject::new("v15-translation-return-event");
    let artifact_path = emit_queue_artifact(&project, "translation_return_event_case");
    let artifact = read_json(&artifact_path);

    assert!(
        artifact["core_obligation_trace"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "return")),
        "Core trace must include normal return terminator events: {artifact}"
    );
    assert!(
        artifact["generated_rust_trace"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "return")),
        "generated trace must preserve normal return terminator events: {artifact}"
    );
}

#[test]
fn fake_source_map_anchor_id_is_rejected_even_when_hashes_are_recomputed() {
    let project = TestProject::new("v15-translation-fake-anchor");
    let artifact_path = emit_queue_artifact(&project, "translation_fake_anchor_case");
    let mut certificate: ProofCertificate =
        serde_json::from_value(read_json(&artifact_path)).expect("certificate should deserialize");
    certificate.generated_rust_trace[0].source_map_anchor.id = "fake-anchor".to_owned();
    for hash in &mut certificate.trace_hashes {
        if hash.id == "generated_rust_trace" {
            hash.hash = trace_material_hash(&certificate.generated_rust_trace)
                .expect("generated trace hash should compute");
        }
    }
    certificate.certificate_material_hash.clear();
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should compute");
    write_json(
        &artifact_path,
        &serde_json::to_value(&certificate).expect("certificate should serialize"),
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "fake source-map anchor ids should reject proof");
    assert!(
        output.combined().contains("source-map anchor"),
        "failure should name source-map anchor validation: {}",
        output.combined()
    );
}

#[test]
fn generated_trace_preserves_real_error_exit_terminator_events() {
    let project = TestProject::new("v15-translation-error-exit-event");
    let artifact_path = emit_artifact(
        &project,
        &error_exit_source("translation_error_exit_event_case"),
        "translation_error_exit_event_case",
    );
    let artifact = read_json(&artifact_path);

    assert!(
        artifact["core_obligation_trace"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "error_exit")),
        "Core trace must include real error-exit terminator events: {artifact}"
    );
    assert!(
        artifact["generated_rust_trace"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "error_exit")),
        "generated trace must preserve real error-exit terminator events: {artifact}"
    );
}

#[test]
fn missing_source_map_rejects_generated_trace_validation() {
    let project = TestProject::new("v15-translation-missing-map");
    let artifact_path = emit_queue_artifact(&project, "translation_missing_map_case");
    let map_path = project
        .find_files_with_ext("map")
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "main.kobo.map"))
        .expect("proof emit should write a source map");
    std::fs::remove_file(&map_path).expect("source map should be removable");

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "missing source map should reject generated trace validation",
    );
    assert!(
        output.combined().contains("source-map anchor")
            || output.combined().contains("translation validation"),
        "failure should name source-map or translation validation: {}",
        output.combined()
    );
}

#[test]
fn source_map_lowering_trace_tamper_rejects_validation() {
    let project = TestProject::new("v15-translation-lowering-trace-tamper");
    let artifact_path = emit_queue_artifact(&project, "translation_lowering_tamper_case");
    let map_path = project
        .find_files_with_ext("map")
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "main.kobo.map"))
        .expect("proof emit should write a source map");
    let mut source_map = read_json(&map_path);
    source_map["lowering_trace"]
        .as_array_mut()
        .expect("lowering trace should be mutable")
        .retain(|event| event["kind"] != "discharge");
    write_json(&map_path, &source_map);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "tampered source-map lowering trace should reject validation",
    );
    assert!(
        output.combined().contains("missing generated event")
            || output.combined().contains("source-map anchor")
            || output.combined().contains("translation validation"),
        "failure should name lowering-trace mismatch: {}",
        output.combined()
    );
}

#[test]
fn source_map_extra_lowering_trace_event_rejects_validation() {
    let project = TestProject::new("v15-translation-extra-lowering-event");
    let artifact_path = emit_queue_artifact(&project, "translation_extra_lowering_case");
    let map_path = project
        .find_files_with_ext("map")
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "main.kobo.map"))
        .expect("proof emit should write a source map");
    let mut source_map = read_json(&map_path);
    let mut extra = source_map["lowering_trace"][0].clone();
    extra["id"] = "lowering-extra-proof-event".into();
    extra["core_event_id"] = "core-extra-proof-event".into();
    extra["order"] = 999.into();
    source_map["lowering_trace"]
        .as_array_mut()
        .expect("lowering trace should be mutable")
        .push(extra);
    write_json(&map_path, &source_map);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "extra source-map lowering event should reject proof",
    );
    assert!(
        output.combined().contains("extra generated event")
            || output.combined().contains("source-map anchor")
            || output.combined().contains("translation validation"),
        "failure should name extra lowering-trace evidence: {}",
        output.combined()
    );
}

#[test]
fn source_map_lowering_trace_order_tamper_rejects_validation() {
    let project = TestProject::new("v15-translation-lowering-order-tamper");
    let artifact_path = emit_queue_artifact(&project, "translation_lowering_order_case");
    let map_path = project
        .find_files_with_ext("map")
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "main.kobo.map"))
        .expect("proof emit should write a source map");
    let mut source_map = read_json(&map_path);
    let trace = source_map["lowering_trace"]
        .as_array_mut()
        .expect("lowering trace should be mutable");
    trace[0]["order"] = 99.into();
    write_json(&map_path, &source_map);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "source-map lowering order tamper should reject validation",
    );
    assert!(
        output.combined().contains("source-map anchor")
            || output.combined().contains("translation validation"),
        "failure should name lowering order mismatch: {}",
        output.combined()
    );
}
