use kobo_driver::scenario::build_scenario_program;
use kobo_driver::test_utils::{compile_to_kir, run_codegen_for_source_with_policy};
use kobo_driver::KoboConfig;

fn source_with_name(name: &str) -> String {
    format!(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery_{name} {{}}

fn handle_{name}() {{
    let msg = Delivery_{name} {{}};
    msg.ack();
}}
"#
    )
}

#[test]
fn scenario_model_is_derived_from_compiler_facts_not_fixture_names() {
    let suffix = format!("case_{}", std::process::id());
    let source = source_with_name(&suffix);
    let config = KoboConfig::default();
    let compiled = compile_to_kir(&source, &config).expect("compile to KIR");
    assert!(
        !compiled.kir.scenario_programs().is_empty(),
        "KIR must own scenario programs before driver selection"
    );
    let codegen =
        run_codegen_for_source_with_policy(&source, "default").expect("codegen artifacts");

    let scenario = build_scenario_program(
        &codegen,
        &format!("handle_{suffix}"),
        "test-source-hash".to_owned(),
    )
    .expect("scenario program");

    assert!(
        scenario
            .operations
            .iter()
            .any(|op| matches!(op.kind, kobo_ir::ScenarioOpKind::CreateObligation { .. })),
        "must-call owner must become a scenario acquire operation"
    );
    assert!(
        scenario
            .operations
            .iter()
            .any(|op| matches!(op.kind, kobo_ir::ScenarioOpKind::Discharge { ref action, .. } if action == "ack")),
        "must-call ack action must become a scenario terminal operation"
    );
    assert!(
        scenario
            .operations
            .iter()
            .all(|op| op.span.file_id == scenario.file_id),
        "scenario operations must retain source-mapped spans"
    );
}

#[test]
fn scenario_lowering_records_whole_program_recursive_sccs() {
    let source = r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {}

fn open(tx: Transaction, retry: bool) {
    step(tx, retry);
}

fn step(tx: Transaction, retry: bool) {
    if retry {
        open(tx, false);
    } else {
        tx.commit();
    }
}

fn transaction_entry() {
    let tx = Transaction {};
    open(tx, true);
}
"#;

    let codegen = run_codegen_for_source_with_policy(source, "checked").expect("codegen artifacts");
    let scenario =
        build_scenario_program(&codegen, "transaction_entry", "test-source-hash".to_owned())
            .expect("scenario program");

    assert!(
        scenario
            .coverage
            .call_graph_sccs
            .iter()
            .any(|component| component.is_recursive
                && component.functions == vec!["open".to_owned(), "step".to_owned()]),
        "scenario lowering must be backed by a whole-program SCC analysis: {:?}",
        scenario.coverage.call_graph_sccs
    );
    assert!(
        scenario
            .operations
            .iter()
            .any(|operation| { matches!(&operation.kind, kobo_ir::ScenarioOpKind::Loop) }),
        "recursive SCC re-entry should be represented as a modeled loop boundary"
    );
    assert!(
        !scenario
            .coverage
            .unsupported_constructs
            .iter()
            .any(|construct| construct.contains("recursion depth")),
        "SCC lowering must not rely on a bounded helper-call recursion guard: {:?}",
        scenario.coverage.unsupported_constructs
    );
}
