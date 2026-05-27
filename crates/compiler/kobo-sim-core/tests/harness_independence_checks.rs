use kobo_ir::{
    FileId, KoboSpan, ScenarioCoverageFacts, ScenarioOp, ScenarioOpKind, ScenarioProgram,
};
use kobo_sim_core::{run_full_depth_from_program, EngineMode, ScenarioOptions};

#[test]
fn harness_trace_comes_from_executed_generated_rust() {
    let nonce = format!("nonce_{}", std::process::id());
    let generated_rust = format!(
        r##"
fn main() {{
    println!("{{}}", r#"KOBO_EVENT:{{"kind":"nonce","label":"{nonce}","value":null}}"#);
    println!("{{}}", r#"KOBO_EVENT:{{"kind":"return","label":null,"value":null}}"#);
}}
"##
    );

    let program = ScenarioProgram {
        file_id: FileId(0),
        target: "main".to_owned(),
        source_hash: format!("source-hash-{nonce}"),
        operations: vec![ScenarioOp {
            span: KoboSpan::generated(FileId(0)),
            kind: ScenarioOpKind::Return,
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    let run = run_full_depth_from_program(
        &program,
        &generated_rust,
        &ScenarioOptions::default(),
        EngineMode::Both,
    )
    .expect("full-depth run");

    assert_eq!(run.digest.harness_engine, "generated-rust-process");
    assert!(run.digest.generated_rust_hash.is_some());
    assert!(run.digest.harness_manifest_hash.is_some());
    assert_eq!(run.digest.harness_exit_code, Some(0));
    assert!(
        run.events
            .iter()
            .any(|event| event.label.as_deref() == Some(nonce.as_str())),
        "captured events must include the runtime nonce emitted by the generated harness"
    );
}
