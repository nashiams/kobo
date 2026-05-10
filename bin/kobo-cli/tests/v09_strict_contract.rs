mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_mentions_line, assert_not_contains, assert_success,
    one_based_line_of, path_arg, run_kobo, s, TestProject,
};

const UNRESOLVED_DEBT: &str = r#"
fn main() {
    let order = String::from("order-7");
    let first = &order;
    let moved = order;
    println!("{} {}", first, moved);
}
"#;

const FIXED_DEBT: &str = r#"
fn main() {
    let order = String::from("order-7");
    let first = order.clone();
    let moved = order;
    println!("{} {}", first, moved);
}
"#;

#[test]
fn release_rejects_unresolved_owned_debt_that_dev_records() {
    let project = TestProject::new("release-rejects-debt");
    let file = project.main_file(UNRESOLVED_DEBT);

    let dev = run_kobo(
        &[s("check"), s("--profile"), s("dev"), path_arg(&file)],
        &project.root,
    );
    let release = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&dev, "dev profile should record debt without blocking");
    assert_failure(
        &release,
        "release profile must reject unresolved ownership debt",
    );
    let text = release.combined();
    assert_contains(
        &text,
        "K000",
        "release failure must use registered K000x code",
    );
    assert_contains(
        &text,
        "ownership",
        "release failure must name ownership debt",
    );
    assert_contains(
        &text,
        "src/main.kobo",
        "release failure must use source span",
    );
}

#[test]
fn release_ownership_diagnostic_span_tracks_shifted_source() {
    let project = TestProject::new("release-shifted-span");
    let compact_source = r#"
fn main() {
    let order = String::from("order-7");
    let first = &order;
    let moved = order;
    println!("{} {}", first, moved);
}
"#;
    let shifted_source = r#"



fn main() {
    let order = String::from("order-7");
    let first = &order;
    let moved = order;
    println!("{} {}", first, moved);
}
"#;
    let compact_line = one_based_line_of(compact_source, "let moved = order");
    let shifted_line = one_based_line_of(shifted_source, "let moved = order");
    assert_ne!(
        compact_line, shifted_line,
        "fixture must shift the move line"
    );
    let compact_file = project.write("src/compact.kobo", compact_source);
    let shifted_file = project.write("src/shifted.kobo", shifted_source);

    let compact = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--error-format=json"),
            path_arg(&compact_file),
        ],
        &project.root,
    );
    let shifted = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--error-format=json"),
            path_arg(&shifted_file),
        ],
        &project.root,
    );

    assert_failure(&compact, "compact ownership debt should fail release");
    assert_failure(&shifted, "shifted ownership debt should fail release");
    assert_mentions_line(
        &compact,
        compact_line,
        "compact diagnostic must point at actual moved value",
    );
    assert_mentions_line(
        &shifted,
        shifted_line,
        "shifted diagnostic must move with source offset",
    );
}

#[test]
fn release_accepts_fixed_debt_and_emits_clean_rust_without_hidden_runtime() {
    let project = TestProject::new("release-clean-rust");
    let file = project.main_file(FIXED_DEBT);

    let output = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--emit-rust"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "fixed release build should succeed");
    let text = output.combined();
    assert_contains(
        &text,
        ".rs",
        "release build should expose generated Rust path",
    );
    assert_not_contains(
        &text,
        "kobo-runtime",
        "release output must not depend on hidden Kobo runtime",
    );
}

#[test]
fn strict_alias_rejects_same_debt_as_release_profile() {
    let project = TestProject::new("strict-rejects-same");
    let file = project.main_file(UNRESOLVED_DEBT);

    let release = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    let strict = run_kobo(
        &[
            s("build"),
            s("--strict"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&release, "release must reject unresolved debt");
    assert_failure(&strict, "strict alias must reject unresolved debt");
    assert_eq!(
        release.status.code(),
        strict.status.code(),
        "`--strict` and `--profile release` should have equivalent exit status"
    );
}

#[test]
fn audit_classifies_mechanical_structural_and_unknown_debt() {
    let project = TestProject::new("audit-tiers");
    let file = project.main_file(
        r#"
extern crate unknown_runtime;

struct Cart {
    name: String,
    total: u64,
}

fn mechanical() {
    let name = String::from("n");
    let other = name.clone();
    println!("{}", other);
}

fn structural(flag: bool) {
    let mut cart = Cart { name: String::from("cart"), total: 0 };
    let view = &cart.name;
    if flag {
        cart.total += 1;
    }
    println!("{}", view);
}

fn unknown() {
    unknown_runtime::opaque();
}
"#,
    );

    let output = run_kobo(
        &[
            s("inspect"),
            s("--profile"),
            s("release"),
            s("--audit=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "release audit should be inspectable");
    let text = output.combined();
    for tier in ["tier\":1", "tier\":2", "tier\":3"] {
        assert_contains(&text, tier, "audit must include all three tiers");
    }
    assert_contains(&text, "src/main.kobo", "audit must preserve source spans");
}

#[test]
fn field_granularity_wraps_only_proven_field_and_perf_reports_real_stats() {
    let project = TestProject::new("field-perf");
    let file = project.main_file(
        r#"
struct Session {
    shared_count: u64,
    immutable_name: String,
}

fn bump(session: &mut Session) {
    session.shared_count += 1;
}

fn read_name(session: &Session) {
    println!("{}", session.immutable_name);
}
"#,
    );

    let build = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("release"),
            s("--emit-rust"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &build,
        "release build should succeed for field-granularity fixture",
    );
    let build_text = build.combined();
    assert_contains(
        &build_text,
        "shared_count",
        "generated output must identify wrapped field",
    );
    assert!(
        !build_text.contains("DiagOwner<Session>"),
        "field granularity must not wrap whole struct when only one field needs it:\n{build_text}"
    );

    let perf = run_kobo(
        &[s("perf"), path_arg(&file), s("--format=json")],
        &project.root,
    );
    assert_success(&perf, "kobo perf should report real analysis stats");
    let perf_text = perf.combined();
    for needle in ["borrow_count", "contention", "hot_paths"] {
        assert_contains(&perf_text, needle, "perf must expose real DiagOwner stats");
    }
}
