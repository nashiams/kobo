mod cli_test_support;
mod proof_test_support;

use kobo_proof::{certificate_material_hash, ProofCertificate, TranslationValidationStatus};
use proof_test_support::{
    assert_success, bounded_source, emit_artifact, first_json, path_arg, queue_loop_source,
    read_json, run_kobo, s, TestProject,
};
use std::fs;
use std::path::Path;

#[test]
fn cli_text_and_json_agree_on_bounded_proof_wording() {
    let project = TestProject::new("translation-wording-bounded-proof");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "wording_bounded_case",
            "complete",
            24,
            24,
            Some("single_thread|work_stealing|priority|random"),
            Some("none|timeout|crash"),
            Some("none|await_recv"),
        ),
        "wording_bounded_case",
    );

    let json_output = run_kobo(
        &[
            s("proof"),
            s("verify"),
            s("--json"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_success(&json_output, "proof verify --json should succeed");
    let json = first_json(&json_output, "proof verify json");
    let wording = json["bounded_wording"][0]
        .as_str()
        .expect("bounded wording should be a string");
    assert_eq!(
        wording,
        "bounded proof: all 24 histories explored under declared bounds"
    );

    let text_output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&text_output, "proof verify should succeed");
    assert!(
        text_output.combined().contains(wording),
        "text output must repeat JSON bounded wording: {}",
        text_output.combined()
    );
}

#[test]
fn cli_text_and_json_agree_on_evidence_only_wording() {
    let project = TestProject::new("translation-wording-evidence-only");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "wording_evidence_case",
            "sampled",
            8,
            24,
            Some("single_thread"),
            Some("none"),
            Some("none"),
        ),
        "wording_evidence_case",
    );

    let json_output = run_kobo(
        &[
            s("proof"),
            s("verify"),
            s("--json"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_success(&json_output, "proof verify --json should succeed");
    let json = first_json(&json_output, "proof verify json");
    let wording = json["bounded_wording"][0]
        .as_str()
        .expect("bounded wording should be a string");
    assert!(wording.contains("evidence only"));

    let text_output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&text_output, "proof verify should succeed");
    assert!(
        text_output.combined().contains(wording),
        "text output must repeat JSON evidence wording: {}",
        text_output.combined()
    );
}

#[test]
fn core_only_wording_excludes_generated_rust_claims() {
    let project = TestProject::new("translation-wording-core-only");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("wording_core_only_case", "delivery", "queue"),
        "wording_core_only_case",
    );
    let mut certificate: ProofCertificate =
        serde_json::from_value(read_json(&artifact_path)).expect("certificate should deserialize");
    certificate.generated_rust_trace.clear();
    certificate.translation_validation.status = TranslationValidationStatus::CoreOnly;
    certificate.translation_validation.mismatches.clear();
    certificate.certificate_material_hash.clear();
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should compute");
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&certificate).expect("certificate should serialize"),
    )
    .expect("certificate should write");

    let text_output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&text_output, "core-only proof artifact should verify");
    let text = text_output.combined();
    assert!(text.contains("Core proof only"), "{text}");
    assert!(
        !text.contains("translation validated"),
        "core-only proof must not claim generated trace validation: {text}"
    );
}

#[test]
fn translation_validated_wording_names_trace_preservation_only() {
    let project = TestProject::new("translation-wording-translation");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("wording_translation_case", "delivery", "queue"),
        "wording_translation_case",
    );

    let text_output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&text_output, "translation-validated artifact should verify");
    let text = text_output.combined();
    assert!(text.contains("translation validated"), "{text}");
    assert!(
        !text.contains("generated Rust binary behavior is proven")
            && !text.contains("validated Rust"),
        "translation wording must not overclaim Rust semantics: {text}"
    );
}

#[test]
fn readme_documents_proof_evidence_and_translation_boundaries() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme_path = manifest_dir.join("..").join("..").join("README.md");
    let readme = fs::read_to_string(&readme_path).expect("README should be readable");

    assert!(readme.contains("proof: modeled Core obligation flow"));
    assert!(readme.contains("bounded proof: all <N> histories explored under declared bounds"));
    assert!(readme.contains("evidence only"));
    assert!(readme.contains("translation validated"));
    assert!(
        readme.contains("does not prove generated Rust binary behavior"),
        "README must state the generated Rust proof boundary"
    );
}
