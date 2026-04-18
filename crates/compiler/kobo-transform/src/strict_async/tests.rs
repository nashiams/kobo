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
    let violations = check_strict_async(&kir, KoboMode::Script);
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
    let violations = check_strict_async(&kir, KoboMode::Script);
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
    let violations = check_strict_async(&kir, KoboMode::Strict);
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
    let violations = check_strict_async(&kir, KoboMode::Script);
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
    let violations = check_strict_async(&kir, KoboMode::Script);
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
    let violations = check_strict_async(&kir, KoboMode::Script);
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
    let script_violations = check_strict_async(&kir, KoboMode::Script);
    let checked_violations = check_strict_async(&kir, KoboMode::Checked);
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
    let violations = check_strict_async(&kir, KoboMode::Strict);
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
    let violations = check_strict_async(&kir, KoboMode::Strict);
    assert!(
        violations.is_empty(),
        "strict mode sync fn should have no async violations"
    );
}
