mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_mentions_line, assert_not_contains, assert_success,
    one_based_line_of, path_arg, run_kobo, s, TestProject,
};

fn inspect_source(label: &str, source: &str) -> String {
    let project = TestProject::new(label);
    let file = project.main_file(source);
    let output = run_kobo(&[s("inspect"), path_arg(&file)], &project.root);
    assert_success(&output, "parallel fixture should inspect");
    output.combined()
}

#[test]
fn parallel_cpu_loop_lowers_to_rayon_par_iter() {
    let generated = inspect_source(
        "parallel-safe-loop",
        r#"
fn cpu_hash(value: u64) -> u64 {
    value.wrapping_mul(31).rotate_left(3)
}

fn crunch(values: Vec<u64>) {
    #[kobo::parallel]
    for value in values.iter() {
        let _hashed = cpu_hash(*value);
    }
}
"#,
    );

    assert_contains(
        &generated,
        "use rayon::prelude::*;",
        "safe parallel loop should import Rayon prelude",
    );
    assert_contains(
        &generated,
        "values.par_iter()",
        "safe CPU-bound loop should lower iter() to par_iter()",
    );
}

#[test]
fn parallel_loop_requires_send_sync_facts() {
    let unsafe_project = TestProject::new("parallel-rc-reject");
    let unsafe_source = r#"
use std::rc::Rc;

fn crunch(values: Vec<u64>) {
    let state = Rc::new(1_u64);
    #[kobo::parallel]
    for value in values.iter() {
        let _seen = *value + *state;
    }
}
"#;
    let unsafe_file = unsafe_project.main_file(unsafe_source);
    let unsafe_output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&unsafe_file)],
        &unsafe_project.root,
    );
    assert_failure(
        &unsafe_output,
        "parallel loop capturing Rc state should be rejected",
    );
    assert_contains(
        &unsafe_output.combined(),
        "state",
        "diagnostic should name the non-Send captured variable",
    );

    let generated = inspect_source(
        "parallel-arc-allowed",
        &unsafe_source
            .replace("std::rc::Rc", "std::sync::Arc")
            .replace("Rc::new", "Arc::new"),
    );
    assert_contains(
        &generated,
        "values.par_iter()",
        "repairing Rc to Arc should allow Rayon lowering",
    );
}

#[test]
fn parallel_rejects_shared_mutation_with_source_span() {
    let project = TestProject::new("parallel-shared-mutation");
    let source = r#"
fn crunch(values: Vec<u64>) {
    let mut output = Vec::new();
    #[kobo::parallel]
    for value in values.iter() {
        output.push(*value);
    }
}
"#;
    let file = project.main_file(source);
    let output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&file)],
        &project.root,
    );
    let mutation_line = one_based_line_of(source, "output.push");

    assert_failure(&output, "parallel loop mutating shared output should fail");
    assert_contains(
        &output.combined(),
        "output",
        "diagnostic should name the shared mutation target",
    );
    assert_mentions_line(
        &output,
        mutation_line,
        "shared mutation diagnostic should point at the mutation site",
    );
}

#[test]
fn parallel_preserves_serial_order_when_order_is_required() {
    let generated = inspect_source(
        "parallel-serial-order",
        r#"
fn emit(values: Vec<u64>) {
    #[kobo::parallel(order = "serial")]
    for value in values.iter() {
        println!("{}", value);
    }
}
"#,
    );

    assert_not_contains(
        &generated,
        "par_iter",
        "serial-order policy should keep the loop serial",
    );
    assert_contains(
        &generated,
        "parallel-order=serial",
        "inspect output should explain why Rayon was not emitted",
    );
}

#[test]
fn parallel_iterator_chain_passthrough_avoids_wrapper_churn() {
    let generated = inspect_source(
        "parallel-iterator-chain",
        r#"
fn crunch(values: Vec<u64>) {
    #[kobo::parallel]
    for value in values.iter().map(|value| value + 1).filter(|value| *value > 2) {
        let _seen = value;
    }
}
"#,
    );

    assert_contains(
        &generated,
        "values.par_iter().map",
        "iterator chain should stay lazy after parallel lowering",
    );
    assert_not_contains(
        &generated,
        "collect::<Vec",
        "parallel lowering should not materialize an intermediate wrapper",
    );
}

#[test]
fn parallel_requires_explicit_policy_near_ward_boundary() {
    let project = TestProject::new("parallel-ward-boundary");
    let source = r#"
fn crunch(values: Vec<u64>) {
    #[kobo::parallel]
    for value in values.iter() {
        ward.task();
        let _seen = value;
    }
}
"#;
    let file = project.main_file(source);
    let output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&file)],
        &project.root,
    );

    assert_failure(
        &output,
        "parallel loop near ward boundary should require explicit policy",
    );
    assert_contains(
        &output.combined(),
        "policy",
        "diagnostic should request an explicit inside/outside policy",
    );

    let generated = inspect_source(
        "parallel-ward-boundary-policy",
        &source.replace(
            "#[kobo::parallel]",
            "#[kobo::parallel(policy = \"outside\")]",
        ),
    );
    assert_contains(
        &generated,
        "parallel-policy=outside",
        "explicit ward-boundary policy should be inspect-visible",
    );
}

#[test]
fn parallel_diagnostic_explains_exact_blocker() {
    let project = TestProject::new("parallel-exact-blocker");
    let source = r#"
use std::rc::Rc;

fn crunch(values: Vec<u64>) {
    let state = Rc::new(1_u64);
    #[kobo::parallel]
    for value in values.iter() {
        let _seen = *value + *state;
    }
}
"#;
    let file = project.main_file(source);
    let output = run_kobo(
        &[s("inspect"), s("--strict"), path_arg(&file)],
        &project.root,
    );

    assert_failure(&output, "non-Send parallel capture should fail");
    for expected in ["non-Send", "state", "Rc"] {
        assert_contains(
            &output.combined(),
            expected,
            "parallel diagnostic should explain the exact blocker",
        );
    }
}
