mod v09_common;
mod v15_common;

use kobo_proof::{certificate_material_hash, ProofCertificate};
use v15_common::{
    assert_failure, assert_success, branch_leak_loop_source, emit_artifact, path_arg,
    queue_loop_source, read_json, run_kobo, s, TestProject,
};

fn protocol_loop_source(
    scenario_name: &str,
    owner_type: &str,
    owner_binding: &str,
    create_method: &str,
    obligation_type: &str,
    obligation_binding: &str,
    terminal_actions: &[&str],
    discharge_action: &str,
) -> String {
    let terminal_methods = terminal_actions
        .iter()
        .map(|action| format!("    fn {action}(self) {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"
struct {owner_type} {{}}
struct {obligation_type} {{}}

impl {owner_type} {{
    fn {create_method}(&self) -> {obligation_type} {{ {obligation_type} {{}} }}
}}

impl {obligation_type} {{
{terminal_methods}
}}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let {owner_binding} = {owner_type} {{}};
    loop {{
        let {obligation_binding} = {owner_binding}.{create_method}();
        {obligation_binding}.{discharge_action}();
    }}
}}
"#
    )
}

fn assert_inferred_template(
    artifact: &serde_json::Value,
    template_id: &str,
    obligation_kind: &str,
    obligation_binding: &str,
) {
    let invariant = &artifact["loop_invariants"][0];
    assert_eq!(invariant["tier"], "inferred");
    assert_eq!(
        invariant["expression"],
        format!("no_pending({obligation_kind})")
    );
    assert_eq!(invariant["template"]["id"], template_id);
    assert_eq!(invariant["template"]["version"], "0.1");
    assert_eq!(invariant["preservation"], "preserved");
    assert_eq!(invariant["obligations_created"][0], obligation_binding);
}

fn queue_loop_source_with_invariant(scenario_name: &str, expression: &str) -> String {
    queue_loop_source(scenario_name, "delivery", "queue").replace(
        "#[kobo::scenario(profile = \"sync\")]",
        &format!(
            "#[kobo::invariant(expression = \"{expression}\")]\n#[kobo::scenario(profile = \"sync\")]"
        ),
    )
}

fn empty_loop_source_with_invariant(scenario_name: &str, expression: &str) -> String {
    format!(
        r#"
#[kobo::invariant(expression = "{expression}")]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    loop {{
        let _unit = ();
    }}
}}
"#
    )
}

fn two_queue_loops_source(scenario_name: &str) -> String {
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
    loop {{
        let first = queue.recv();
        first.ack();
        break;
    }}
    loop {{
        let second = queue.recv();
        second.ack();
        break;
    }}
}}
"#
    )
}

fn two_queue_while_loops_source(scenario_name: &str) -> String {
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

fn choose() -> bool {{ true }}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let queue = Queue {{}};
    while choose() {{
        let first = queue.recv();
        first.ack();
    }}
    while choose() {{
        let second = queue.recv();
        second.ack();
    }}
}}
"#
    )
}

fn two_labeled_queue_while_loops_with_user_invariant(scenario_name: &str) -> String {
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

fn choose() -> bool {{ true }}

#[kobo::invariant(expression = "no_pending(Delivery)", loop_label = "second")]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let queue = Queue {{}};
    'first: while choose() {{
        let first = queue.recv();
        first.ack();
    }}
    'second: while choose() {{
        let second = queue.recv();
        second.ack();
    }}
}}
"#
    )
}

fn labeled_outer_break_source(scenario_name: &str) -> String {
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
    'outer: loop {{
        loop {{
            let delivery = queue.recv();
            delivery.ack();
            break 'outer;
        }}
    }}
}}
"#
    )
}

fn mixed_labeled_branch_control_source(scenario_name: &str) -> String {
    format!(
        r#"
fn choose() -> bool {{
    std::env::args().next().is_some()
}}

#[kobo::bounded(histories = "4", expected = "4", completeness = "complete", scheduler = "fifo|round_robin", fault = "none|timeout", cancellation = "none", queue_capacity = "1", message_count = "1", retry_attempts = "1", timeout_paths = "1", external_boundary_recordings = "0")]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    'outer: loop {{
        loop {{
            if choose() {{
                break 'outer;
            }} else {{
                continue;
            }}
        }}
    }}
}}
"#
    )
}

fn loop_control_flow_source(scenario_name: &str, control_flow: &str) -> String {
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

fn choose() -> bool {{ true }}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let queue = Queue {{}};
    loop {{
        let delivery = queue.recv();
        {control_flow}
        delivery.ack();
    }}
}}
"#
    )
}

fn while_queue_loop_source(scenario_name: &str) -> String {
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

fn choose() -> bool {{ true }}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let queue = Queue {{}};
    while choose() {{
        let delivery = queue.recv();
        delivery.ack();
    }}
}}
"#
    )
}

#[test]
fn queue_loop_infers_template_invariant() {
    let project = TestProject::new("v15-loop-invariant-queue");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("queue_invariant_case", "delivery", "queue"),
        "queue_invariant_case",
    );
    let artifact = read_json(&artifact_path);
    let invariant = &artifact["loop_invariants"][0];

    assert_eq!(invariant["tier"], "inferred");
    assert_eq!(invariant["expression"], "no_pending(Delivery)");
    assert_eq!(invariant["template"]["id"], "queue_delivery");
    assert_eq!(invariant["template"]["version"], "0.1");
    assert_eq!(invariant["preservation"], "preserved");
    assert!(
        invariant["back_edge_states"]
            .as_array()
            .is_some_and(|states| states
                .iter()
                .all(|state| state["state"] != "owned" && state["state"] != "branch_unresolved")),
        "back-edge must not carry unresolved per-iteration obligations: {artifact}"
    );
}

#[test]
fn renamed_queue_loop_proves_the_same_template() {
    let project = TestProject::new("v15-loop-invariant-renamed");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("renamed_queue_case", "message", "inbox"),
        "renamed_queue_case",
    );
    let artifact = read_json(&artifact_path);
    let invariant = &artifact["loop_invariants"][0];

    assert_eq!(invariant["expression"], "no_pending(Delivery)");
    assert_eq!(invariant["template"]["id"], "queue_delivery");
    assert_eq!(
        invariant["obligations_created"][0], "message",
        "binding rename should not change the inferred protocol template: {artifact}"
    );
}

#[test]
fn branch_leak_reaching_back_edge_rejects_proof() {
    let project = TestProject::new("v15-loop-invariant-branch-leak");
    let source_file = project.main_file(&branch_leak_loop_source("branch_leak_case"));
    let artifact_path = project.root.join("branch_leak_case.kproof");

    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&source_file),
            s("--target"),
            s("branch_leak_case"),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );

    assert_failure(&output, "branch leak should reject proof emission");
    assert!(
        output.combined().contains("back-edge")
            || output.combined().contains("BranchUnresolved")
            || output.combined().contains("branch_unresolved")
            || output.combined().contains("CFG edge transition mismatch"),
        "failure should name the back-edge, join, or branch-unresolved obligation: {}",
        output.combined()
    );
}

#[test]
fn continue_before_discharge_rejects_back_edge_proof() {
    let project = TestProject::new("v15-loop-invariant-continue");
    let source_file = project.main_file(&loop_control_flow_source(
        "continue_leak_case",
        "if choose() { continue; }",
    ));
    let artifact_path = project.root.join("continue_leak_case.kproof");

    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&source_file),
            s("--target"),
            s("continue_leak_case"),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "continue before discharge should reject proof emission",
    );
    assert!(
        output.combined().contains("back-edge")
            || output.combined().contains("continue")
            || output.combined().contains("BranchUnresolved")
            || output.combined().contains("branch_unresolved"),
        "failure should name continue/back-edge unresolved flow: {}",
        output.combined()
    );
}

#[test]
fn early_break_before_discharge_rejects_loop_proof() {
    let project = TestProject::new("v15-loop-invariant-break");
    let source_file = project.main_file(&loop_control_flow_source(
        "early_break_leak_case",
        "if choose() { break; }",
    ));
    let artifact_path = project.root.join("early_break_leak_case.kproof");

    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&source_file),
            s("--target"),
            s("early_break_leak_case"),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "early break before discharge should reject proof emission",
    );
    assert!(
        output.combined().contains("back-edge")
            || output.combined().contains("break")
            || output.combined().contains("BranchUnresolved")
            || output.combined().contains("branch_unresolved"),
        "failure should name break/back-edge unresolved flow: {}",
        output.combined()
    );
}

#[test]
fn break_edges_are_loop_exits_not_back_edges() {
    let project = TestProject::new("v15-loop-invariant-break-exit");
    let artifact_path = emit_artifact(
        &project,
        &two_queue_loops_source("break_exit_case"),
        "break_exit_case",
    );
    let artifact = read_json(&artifact_path);
    let loop_facts = artifact["core"]["loop_facts"]
        .as_array()
        .expect("loop facts should be present");
    let exit_facts = artifact["core"]["loop_exit_facts"]
        .as_array()
        .expect("loop exit facts should be present");

    assert!(
        loop_facts
            .iter()
            .all(|fact| fact["source_span"]["snippet"] != "break;"),
        "break statements must not be represented as loop back-edges: {artifact}"
    );
    assert_eq!(
        exit_facts.len(),
        2,
        "each break statement should produce a distinct loop exit fact: {artifact}"
    );
    assert!(
        exit_facts
            .iter()
            .all(|fact| fact["exit_target"] != fact["entry_block"]),
        "break loop exits should target the post-loop block or modeled function exit, not the loop entry: {artifact}"
    );
    assert!(
        exit_facts.iter().any(|fact| fact["exit_target"]
            .as_str()
            .is_some_and(|target| target.starts_with("bb"))),
        "a break followed by more code must target the post-loop block: {artifact}"
    );
    assert!(
        exit_facts
            .iter()
            .any(|fact| fact["exit_target"] == "break_exit"),
        "a final break should retain a modeled break_exit target: {artifact}"
    );
}

#[test]
fn labeled_break_targets_the_named_outer_loop() {
    let project = TestProject::new("v15-loop-invariant-labeled-break");
    let artifact_path = emit_artifact(
        &project,
        &labeled_outer_break_source("labeled_outer_break_case"),
        "labeled_outer_break_case",
    );
    let artifact = read_json(&artifact_path);
    let exit_facts = artifact["core"]["loop_exit_facts"]
        .as_array()
        .expect("loop exit facts should be present");
    let outer_exit = exit_facts
        .iter()
        .find(|fact| fact["loop_label"] == "outer")
        .unwrap_or_else(|| panic!("labeled break must name the outer loop: {artifact}"));

    assert_eq!(outer_exit["exit_kind"], "break");
    assert!(
        artifact["core"]["cfg_edges"]
            .as_array()
            .expect("Core edges should be present")
            .iter()
            .all(|edge| {
                edge["to"]
                    .as_str()
                    .is_some_and(|target| !target.starts_with("loop_exit:"))
                    && edge["loop_id"].as_str().is_some_and(|id| !id.is_empty())
                        == edge["loop_edge_kind"].as_str().is_some()
            }),
        "loop control metadata should be typed on Core edges, not encoded in the target: {artifact}"
    );
}

#[test]
fn mixed_labeled_branch_control_does_not_fabricate_back_edges() {
    let project = TestProject::new("v15-loop-invariant-mixed-labeled-control");
    let artifact_path = emit_artifact(
        &project,
        &mixed_labeled_branch_control_source("mixed_labeled_control_case"),
        "mixed_labeled_control_case",
    );
    let artifact = read_json(&artifact_path);
    let cfg_edges = artifact["core"]["cfg_edges"]
        .as_array()
        .expect("Core edges should be present");
    let loop_facts = artifact["core"]["loop_facts"]
        .as_array()
        .expect("loop facts should be present");
    let exit_facts = artifact["core"]["loop_exit_facts"]
        .as_array()
        .expect("loop exit facts should be present");

    assert!(
        cfg_edges
            .iter()
            .any(|edge| edge["loop_edge_kind"] == "continue"),
        "the unlabeled continue branch should remain a typed continue edge: {artifact}"
    );
    assert!(
        exit_facts
            .iter()
            .any(|fact| fact["loop_label"] == "outer" && fact["exit_kind"] == "break"),
        "the labeled break branch should remain an outer loop exit fact: {artifact}"
    );
    assert!(
        cfg_edges
            .iter()
            .all(|edge| edge["loop_edge_kind"] != "back_edge"),
        "a branch where every arm exits by break/continue must not synthesize a fallthrough back-edge: {artifact}"
    );
    assert!(
        loop_facts.iter().all(|fact| fact["loop_label"] != "outer"),
        "outer loop must not receive a fabricated back-edge fact when all inner branch arms transfer control: {artifact}"
    );
}

#[test]
fn while_loop_records_back_edge_facts() {
    let project = TestProject::new("v15-loop-invariant-while");
    let artifact_path = emit_artifact(
        &project,
        &while_queue_loop_source("while_loop_case"),
        "while_loop_case",
    );
    let artifact = read_json(&artifact_path);

    assert!(
        artifact["core"]["loop_facts"]
            .as_array()
            .is_some_and(|facts| !facts.is_empty()),
        "while loops must emit Core loop facts: {artifact}"
    );
    assert!(
        artifact["loop_invariants"]
            .as_array()
            .is_some_and(|invariants| !invariants.is_empty()),
        "while loops must carry proof-grade invariant evidence: {artifact}"
    );
}

#[test]
fn stream_loop_infers_template_before_user_invariant() {
    let project = TestProject::new("v15-loop-invariant-stream");
    let source = protocol_loop_source(
        "stream_loop_case",
        "Stream",
        "stream",
        "next",
        "StreamItem",
        "entry",
        &["consume", "skip"],
        "consume",
    );
    let artifact_path = emit_artifact(&project, &source, "stream_loop_case");
    let artifact = read_json(&artifact_path);

    assert_inferred_template(&artifact, "stream_item", "StreamItem", "entry");
}

#[test]
fn retry_loop_infers_template_before_user_invariant() {
    let project = TestProject::new("v15-loop-invariant-retry");
    let source = protocol_loop_source(
        "retry_loop_case",
        "RetryPolicy",
        "policy",
        "attempt",
        "RetryAttempt",
        "attempt",
        &["succeed", "retry", "give_up"],
        "succeed",
    );
    let artifact_path = emit_artifact(&project, &source, "retry_loop_case");
    let artifact = read_json(&artifact_path);

    assert_inferred_template(&artifact, "retry_attempt", "RetryAttempt", "attempt");
}

#[test]
fn transaction_loop_infers_template_before_user_invariant() {
    let project = TestProject::new("v15-loop-invariant-transaction");
    let source = protocol_loop_source(
        "transaction_loop_case",
        "TransactionManager",
        "manager",
        "begin",
        "Transaction",
        "tx",
        &["commit", "rollback"],
        "commit",
    );
    let artifact_path = emit_artifact(&project, &source, "transaction_loop_case");
    let artifact = read_json(&artifact_path);

    assert_inferred_template(&artifact, "transaction", "Transaction", "tx");
}

#[test]
fn service_request_loop_infers_reply_template_before_user_invariant() {
    let project = TestProject::new("v15-loop-invariant-service");
    let source = protocol_loop_source(
        "service_request_loop_case",
        "Service",
        "service",
        "request",
        "HandlerReply",
        "reply",
        &["reply", "reject", "cancel"],
        "reply",
    );
    let artifact_path = emit_artifact(&project, &source, "service_request_loop_case");
    let artifact = read_json(&artifact_path);

    assert_inferred_template(&artifact, "handler_reply", "HandlerReply", "reply");
}

#[test]
fn explicit_user_invariant_records_user_tier_and_preservation() {
    let project = TestProject::new("v15-loop-invariant-user");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source_with_invariant("user_invariant_case", "no_pending(Delivery)"),
        "user_invariant_case",
    );
    let artifact = read_json(&artifact_path);
    let invariant = &artifact["loop_invariants"][0];

    assert_eq!(invariant["tier"], "user");
    assert_eq!(invariant["expression"], "no_pending(Delivery)");
    assert_eq!(invariant["preservation"], "preserved");
    assert_eq!(invariant["obligations_created"][0], "delivery");
    assert!(
        invariant["loop_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "user invariant must be scoped to a concrete loop id: {artifact}"
    );
    assert_eq!(
        invariant["entry_states"],
        serde_json::json!([]),
        "user invariant must record loop-entry truth evidence: {artifact}"
    );
}

#[test]
fn explicit_user_invariant_is_scoped_to_named_loop() {
    let project = TestProject::new("v15-loop-invariant-user-scope");
    let artifact_path = emit_artifact(
        &project,
        &two_labeled_queue_while_loops_with_user_invariant("user_scoped_loop_case"),
        "user_scoped_loop_case",
    );
    let artifact = read_json(&artifact_path);
    let invariants = artifact["loop_invariants"]
        .as_array()
        .expect("loop invariants should be present");
    let user_invariants = invariants
        .iter()
        .filter(|invariant| invariant["tier"] == "user")
        .collect::<Vec<_>>();

    assert_eq!(
        user_invariants.len(),
        1,
        "user invariant should annotate only the named loop: {artifact}"
    );
    assert_eq!(user_invariants[0]["loop_label"], "second");
    assert_eq!(
        user_invariants[0]["obligations_created"],
        serde_json::json!(["second"])
    );
    assert!(
        invariants
            .iter()
            .any(|invariant| invariant["loop_label"] == "first" && invariant["tier"] == "inferred"),
        "unannotated loop should keep inferred proof evidence: {artifact}"
    );
}

#[test]
fn explicit_user_invariant_rejects_unknown_obligation_kind() {
    let project = TestProject::new("v15-loop-invariant-user-unknown");
    let source_file = project.main_file(&empty_loop_source_with_invariant(
        "unknown_user_case",
        "no_pending(Ghost)",
    ));
    let artifact_path = project.root.join("unknown_user_case.kproof");

    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&source_file),
            s("--target"),
            s("unknown_user_case"),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "unknown user invariant obligation kind should reject proof emission",
    );
    assert!(
        output.combined().contains("unknown obligation kind")
            || output.combined().contains("invariant was not preserved"),
        "failure should name the unknown invariant fact: {}",
        output.combined()
    );
}

#[test]
fn tampered_invariant_template_is_rejected() {
    let project = TestProject::new("v15-loop-invariant-template-tamper");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("template_tamper_case", "delivery", "queue"),
        "template_tamper_case",
    );
    let mut artifact = read_json(&artifact_path);
    artifact["loop_invariants"][0]["template"]["version"] = "stale".into();
    v15_common::write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "stale template must reject proof verification");
    assert!(
        output.combined().contains("template") || output.combined().contains("material hash"),
        "failure should name template evidence or certificate material: {}",
        output.combined()
    );
}

#[test]
fn valid_loop_artifact_verifies_after_invariant_checks() {
    let project = TestProject::new("v15-loop-invariant-verify");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("loop_verify_case", "delivery", "queue"),
        "loop_verify_case",
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_success(&output, "valid loop invariant artifact should verify");
}

#[test]
fn multiple_loops_scope_invariants_to_each_back_edge() {
    let project = TestProject::new("v15-loop-invariant-scoped");
    let artifact_path = emit_artifact(
        &project,
        &two_queue_while_loops_source("multi_loop_scope_case"),
        "multi_loop_scope_case",
    );
    let artifact = read_json(&artifact_path);
    let invariants = artifact["loop_invariants"]
        .as_array()
        .expect("loop invariants should be an array");

    assert_eq!(
        invariants.len(),
        2,
        "both loops need distinct invariants: {artifact}"
    );
    assert_eq!(
        invariants[0]["obligations_created"],
        serde_json::json!(["first"])
    );
    assert_eq!(
        invariants[1]["obligations_created"],
        serde_json::json!(["second"])
    );

    let events = artifact["obligation_events"]
        .as_array()
        .expect("obligation events should be present");
    for (binding, invariant) in [("first", &invariants[0]), ("second", &invariants[1])] {
        let loop_id = invariant["loop_id"]
            .as_str()
            .unwrap_or_else(|| panic!("invariant must carry a typed loop id: {artifact}"));
        let create_event = events
            .iter()
            .find(|event| event["binding"] == binding && event["kind"] == "create")
            .unwrap_or_else(|| panic!("create event for {binding} should be present: {artifact}"));
        assert!(
            create_event["loop_regions"]
                .as_array()
                .is_some_and(|regions| regions.iter().any(|region| region == loop_id)),
            "obligation event for {binding} must carry typed loop-region membership: {artifact}"
        );
    }
}

#[test]
fn tampered_invariant_back_edge_states_are_replayed_from_core() {
    let project = TestProject::new("v15-loop-invariant-state-tamper");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("state_tamper_case", "delivery", "queue"),
        "state_tamper_case",
    );
    let mut certificate: ProofCertificate =
        serde_json::from_value(read_json(&artifact_path)).expect("certificate should deserialize");
    certificate.loop_invariants[0].back_edge_states.clear();
    certificate.certificate_material_hash.clear();
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should compute");
    v15_common::write_json(
        &artifact_path,
        &serde_json::to_value(&certificate).expect("certificate should serialize"),
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "tampered invariant states should reject proof verification",
    );
    assert!(
        output.combined().contains("invariant")
            || output.combined().contains("back-edge")
            || output.combined().contains("replay mismatch"),
        "failure should name invariant/Core state mismatch: {}",
        output.combined()
    );
}

#[test]
fn tampered_user_invariant_entry_states_are_replayed_from_core() {
    let project = TestProject::new("v15-loop-invariant-entry-state-tamper");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source_with_invariant("entry_state_tamper_case", "no_pending(Delivery)"),
        "entry_state_tamper_case",
    );
    let mut certificate: ProofCertificate =
        serde_json::from_value(read_json(&artifact_path)).expect("certificate should deserialize");
    certificate.loop_invariants[0].entry_states = vec![kobo_proof::ObligationState {
        binding: "delivery".to_owned(),
        state: kobo_proof::ObligationStatus::Owned,
    }];
    certificate.certificate_material_hash.clear();
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should compute");
    v15_common::write_json(
        &artifact_path,
        &serde_json::to_value(&certificate).expect("certificate should serialize"),
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "tampered user invariant entry states should reject proof verification",
    );
    assert!(
        output.combined().contains("entry") || output.combined().contains("invariant"),
        "failure should name invariant entry/Core state mismatch: {}",
        output.combined()
    );
}
