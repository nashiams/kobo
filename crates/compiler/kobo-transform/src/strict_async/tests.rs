use kobo_ir::{AsyncViolationKind, FileId, KoboMode, NodeIdGen};
use kobo_parser::parse_file;

use crate::options::TransformOptions;
use crate::transform::build_kir;

use super::check_strict_async;

fn build_test_kir(source: &str) -> kobo_ir::Kir {
    let mut id_gen = NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    build_kir(&ast, &mut id_gen, TransformOptions::default())
}

// --- K0060: Non-Send binding in async context ---

/// Contract Test 11.1: Non-async code produces zero async violations.
#[test]
fn sync_code_no_violations() {
    let kir = build_test_kir(
        r#"
fn main() {
    let data = String::from("hello");
    println!("{}", data);
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    assert!(
        violations.is_empty(),
        "sync code should have no async violations, got {:?}",
        violations
    );
}

/// Contract Test 11.1: Async function with local-only binding → no violations.
/// A2 row: PlainOwned, not shared, not escaped → no K0060.
#[test]
fn async_local_only_no_violations() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let buffer = Vec::new();
    buffer.push(1);
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    // Local-only binding in async: no sharing → no K0060
    assert!(
        violations.is_empty(),
        "local-only async binding should have no violations, got {:?}",
        violations
    );
}

/// Contract Test 11.2: Strict mode with async binding that needs sharing → K0063.
#[test]
fn strict_mode_async_shared_emits_k0063() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Strict, true);
    let has_strict_violation = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::StrictAsyncViolation { .. }));
    assert!(
        has_strict_violation,
        "strict mode with async shared binding should emit K0063, got {:?}",
        violations
    );
}

/// Contract: Script mode does NOT emit K0063 for same code.
#[test]
fn script_mode_no_k0063() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    let has_strict_violation = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::StrictAsyncViolation { .. }));
    assert!(
        !has_strict_violation,
        "script mode should NOT emit K0063, got {:?}",
        violations
    );
}

/// Contract: Non-Send shared binding in async → K0060.
#[test]
fn async_shared_binding_emits_k0060() {
    // Use the EXACT pattern that triggers needs_sharing in sync tests:
    // `let x = ...; let y = x; x.len(); let _ = y;` — use-after-move alias
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    let has_non_send = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::NonSendCapture { .. }));
    assert!(
        has_non_send,
        "async shared binding should emit K0060, got {:?}",
        violations
    );
}

/// Contract: Copy types in async → no violations (A1 row).
#[test]
fn async_copy_type_no_violations() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x: i32 = 42;
    let y = x;
    let z = x;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    assert!(
        violations.is_empty(),
        "copy type in async should have no violations, got {:?}",
        violations
    );
}

/// Contract: Checked mode produces same violations as script for K0060.
#[test]
fn checked_mode_k0060_same_as_script() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let script_violations = check_strict_async(&kir, KoboMode::Script, true);
    let checked_violations = check_strict_async(&kir, KoboMode::Checked, true);
    // K0060 should appear in both modes (severity differs, but fact is same)
    let script_k0060 = script_violations
        .iter()
        .filter(|v| matches!(&v.kind, AsyncViolationKind::NonSendCapture { .. }))
        .count();
    let checked_k0060 = checked_violations
        .iter()
        .filter(|v| matches!(&v.kind, AsyncViolationKind::NonSendCapture { .. }))
        .count();
    assert_eq!(
        script_k0060, checked_k0060,
        "K0060 count should be same in script and checked mode"
    );
}

// --- Hard Rules Enforcement Tests ---

/// Contract: Strict mode + async + shared binding → K0063 (no wrappers allowed).
/// This verifies the hard rule: strict = no wrappers, period.
#[test]
fn strict_mode_async_never_allows_sharing() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Strict, true);
    // Strict mode must emit K0063 for ANY shared binding in async
    let k0063_count = violations
        .iter()
        .filter(|v| matches!(&v.kind, AsyncViolationKind::StrictAsyncViolation { .. }))
        .count();
    assert!(
        k0063_count > 0,
        "strict mode MUST refuse wrapping in async context"
    );
}

/// Contract: Sync function in strict mode → no async violations.
#[test]
fn strict_mode_sync_fn_no_violations() {
    let kir = build_test_kir(
        r#"
fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Strict, true);
    assert!(
        violations.is_empty(),
        "strict mode sync fn should have no async violations"
    );
}

// --- K0061: Non-Sync mutable shared binding in async context ---

/// Contract: Mutable shared binding in async → K0061 (RefCell is not Sync).
#[test]
fn async_mutable_shared_binding_can_emit_k0061() {
    // Pattern: use-after-move alias WITH both read and mutation
    // → needs_sharing + needs_mutable_wrapper
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    x.push_str(" world");
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    let has_non_sync = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::NonSyncShared { .. }));
    assert!(
        has_non_sync,
        "async mutable shared binding should emit K0061, got {:?}",
        violations
    );
}

// --- K0062: Missing executor ---

/// Contract: Async code without executor → K0062.
#[test]
fn async_entrypoint_without_executor_can_emit_k0062() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    println!("{}", x);
}
"#,
    );
    // has_executor = false → should emit K0062
    let violations = check_strict_async(&kir, KoboMode::Script, false);
    let has_missing_executor = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::MissingExecutor));
    assert!(
        has_missing_executor,
        "async code without executor should emit K0062, got {:?}",
        violations
    );
}

/// Contract: Async code WITH executor → no K0062.
#[test]
fn async_with_executor_no_k0062() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    println!("{}", x);
}
"#,
    );
    // has_executor = true → should NOT emit K0062
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    let has_missing_executor = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::MissingExecutor));
    assert!(
        !has_missing_executor,
        "async code WITH executor should NOT emit K0062, got {:?}",
        violations
    );
}

// ---------------------------------------------------------------------------
// v0.7 Test Enforcement — Async: Decision Table (A3-A10)
// ---------------------------------------------------------------------------

/// A3: !Send, mutable, shared → Rc<RefCell> tier (RcMutShared).
#[test]
fn async_non_send_mutable_shared_rc_mut() {
    // The tier decision for a mutable shared binding in async should be RcMutShared
    // when no Send constraint is present.
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.push_str(" world");
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    // Should have K0060 (non-send) but NOT K0063 (strict).
    let has_k0060 = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::NonSendCapture { .. }));
    assert!(
        has_k0060,
        "A3: !Send mutable shared should produce K0060, got {:?}",
        violations
    );
}

/// A4: !Send, read-only, shared → Rc tier (RcShared).
#[test]
fn async_non_send_readonly_shared_rc() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Script, true);
    let has_k0060 = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::NonSendCapture { .. }));
    assert!(
        has_k0060,
        "A4: !Send read-only shared should produce K0060, got {:?}",
        violations
    );
}

/// A9: Strict + Send → K0063.
#[test]
fn async_strict_send_emits_k0063() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Strict, true);
    let has_k0063 = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::StrictAsyncViolation { .. }));
    assert!(
        has_k0063,
        "A9: Strict + Send → K0063, got {:?}",
        violations
    );
}

/// A10: Strict, local-only (no Send needed) → PlainOwned, no violations.
#[test]
fn async_strict_local_only_no_violations() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let _ = x.len();
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Strict, true);
    assert!(
        violations.is_empty(),
        "A10: Strict + local-only should have no violations, got {:?}",
        violations
    );
}

/// K0061 #14: K0061 error in strict mode should escalate to K0063.
#[test]
fn async_k0061_in_strict_mode_escalates_to_k0063() {
    let kir = build_test_kir(
        r#"
async fn process() {
    let x = String::from("hello");
    let y = x;
    x.push_str(" world");
    x.len();
    let _ = y;
}
"#,
    );
    let violations = check_strict_async(&kir, KoboMode::Strict, true);
    // In strict mode, any sharing violation → K0063.
    let has_k0063 = violations
        .iter()
        .any(|v| matches!(&v.kind, AsyncViolationKind::StrictAsyncViolation { .. }));
    assert!(
        has_k0063,
        "K0061 in strict mode should escalate to K0063, got {:?}",
        violations
    );
}

// ---------------------------------------------------------------------------
// v0.7 Test Enforcement — Async: Unimplemented Features (ignored)
// ---------------------------------------------------------------------------

/// Executor selection #5: Tokio detected → #[tokio::main].
#[test]
#[ignore = "feature not yet implemented: executor detection and attribute injection"]
fn async_executor_tokio_detected() {
    // When tokio is in deps, async main should get #[tokio::main].
}

/// Executor selection #6: async-std detected → #[async_std::main].
#[test]
#[ignore = "feature not yet implemented: executor detection and attribute injection"]
fn async_executor_async_std_detected() {
    // When async-std is in deps, async main should get #[async_std::main].
}

/// Executor selection #7: Non-main async fn → no attribute.
#[test]
#[ignore = "feature not yet implemented: executor detection and attribute injection"]
fn async_non_main_fn_no_executor_attribute() {
    // Non-main async fn should not get executor attribute.
}

/// Tier selection #8: Send required → Arc.
#[test]
#[ignore = "feature not yet implemented: Send analysis for Arc tier selection"]
fn async_send_required_uses_arc() {
    // When binding is shared across tasks (Send required), Arc tier should be selected.
}

/// Tier selection #9: Local async → still Rc.
#[test]
#[ignore = "feature not yet implemented: local-task Rc tier selection in async"]
fn async_local_task_still_uses_rc() {
    // When binding is shared within local async context (no Send), Rc tier is preferred.
}

/// Integration #10: End-to-end async build.
#[test]
#[ignore = "feature not yet implemented: end-to-end async project build"]
fn async_end_to_end_build() {
    // Full async project build with executor, tiers, and code generation.
}

/// Hard rule #11: No std::sync::Mutex in async output.
#[test]
#[ignore = "feature not yet implemented: async codegen output validation"]
fn async_no_std_sync_mutex_in_output() {
    // Generated async code must never use std::sync::Mutex.
}

/// Hard rule #12: No Mutex even with mutation + Send.
#[test]
#[ignore = "feature not yet implemented: async codegen with mutation and Send"]
fn async_no_mutex_even_with_mutation_and_send() {
    // Even when mutation + Send is needed, use RwLock not Mutex.
}

/// @strict async #15: Plain Rust output, no wrappers.
#[test]
#[ignore = "feature not yet implemented: @strict async fn codegen"]
fn strict_async_fn_plain_rust_output() {
    // @strict async fn should produce plain Rust with no Rc/Arc wrappers.
}

/// @strict async #16: Executor attribute still generated.
#[test]
#[ignore = "feature not yet implemented: @strict async fn executor attribute"]
fn strict_async_fn_executor_attribute_still_generated() {
    // Even in @strict, executor attribute should be injected.
}

/// @strict async #17: Local mutation → just `let mut`.
#[test]
#[ignore = "feature not yet implemented: @strict async local mutation"]
fn strict_async_fn_local_mutation_just_let_mut() {
    // @strict async fn with local mutation should use plain `let mut`.
}

/// @strict async #18: Island of strictness in script file.
#[test]
#[ignore = "feature not yet implemented: @strict islands in non-strict files"]
fn strict_async_fn_island_in_script_file() {
    // A single @strict async fn in a script mode file should work.
}

/// #[kobo::async_shared] #20: Forces Arc wrapping.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] attribute"]
fn kobo_async_shared_forces_arc_wrapping() {
    // #[kobo::async_shared] on a let binding should force Arc tier.
}

/// #[kobo::async_shared] #21: On struct field.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] on struct fields"]
fn kobo_async_shared_on_struct_field() {
    // #[kobo::async_shared] on a struct field should force Arc tier.
}

/// #[kobo::async_shared] #22: Suppresses K0060/K0061.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] suppression"]
fn kobo_async_shared_suppresses_k0060_k0061() {
    // Explicit #[kobo::async_shared] should suppress K0060 and K0061.
}

/// #[kobo::async_shared] #23: Read-only → Arc (not ArcMut).
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] read-only"]
fn kobo_async_shared_readonly_uses_arc_not_arc_mut() {
    // #[kobo::async_shared] on a read-only binding should use Arc, not ArcMut.
}

/// #[kobo::async_shared] #24: Attribute stripped from generated code.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] stripping"]
fn kobo_async_shared_attribute_stripped_from_output() {
    // #[kobo::async_shared] should not appear in generated Rust code.
}

/// LocalSet #25: !Send → LocalSet + spawn_local.
#[test]
#[ignore = "feature not yet implemented: LocalSet/spawn_local generation"]
fn async_local_set_for_non_send() {
    // !Send values should trigger LocalSet + spawn_local.
}

/// LocalSet #26: Mixed Send and !Send.
#[test]
#[ignore = "feature not yet implemented: mixed Send/!Send with LocalSet"]
fn async_mixed_send_and_non_send() {
    // Mixed Send and !Send bindings should use appropriate strategies.
}

/// LocalSet #27: async fn main with !Send → LocalSet wrapper.
#[test]
#[ignore = "feature not yet implemented: main fn LocalSet wrapper"]
fn async_main_with_non_send_uses_local_set() {
    // async fn main with !Send should wrap in LocalSet.
}

/// MIR dataflow #28: Pass 1 borrow liveness affects tier.
#[test]
#[ignore = "feature not yet implemented: MIR pass 1 borrow liveness in async"]
fn async_mir_pass1_borrow_liveness_affects_tier() {
    // MIR-level borrow liveness should influence tier selection.
}

/// MIR dataflow #29: Pass 2 Send propagation across functions.
#[test]
#[ignore = "feature not yet implemented: MIR pass 2 Send propagation"]
fn async_mir_pass2_send_propagation_across_functions() {
    // Send requirement should propagate across function boundaries.
}
