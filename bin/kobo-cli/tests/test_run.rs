use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct FixtureCase {
    root: PathBuf,
    fixture_path: PathBuf,
}

impl FixtureCase {
    fn new(name: &str, fixture_name: &str) -> Self {
        let workspace_root = workspace_root();
        let unique_id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = workspace_root
            .join("target-test-fixtures")
            .join(format!("{name}-{}-{unique_id}", std::process::id()));

        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }

        fs::create_dir_all(&root).expect("temp fixture dir should be creatable");

        let source_fixture = workspace_root
            .join("tests")
            .join("fixtures")
            .join(fixture_name);
        let fixture_path = root.join(fixture_name);
        fs::copy(&source_fixture, &fixture_path).expect("fixture should copy");

        Self { root, fixture_path }
    }
}

impl Drop for FixtureCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn run_hello_fixture_prints_expected_output() {
    let case = FixtureCase::new("run-hello", "hello.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "alice, bob");
}

#[test]
fn inspect_hello_fixture_matches_snapshot() {
    let case = FixtureCase::new("inspect-hello", "hello.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("let names = vec![\"alice\", \"bob\"];"));
    assert!(!output.stdout.contains("Rc<RefCell<Vec<&str>>>"));
    assert!(output.stdout.contains("// kobo: names @ line 2"));
    assert!(case.fixture_path.with_extension("kobo.map").is_file());

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__hello_annotated", output.stdout);
    });
}

#[test]
fn dump_hello_fixture_reports_expected_tiers() {
    let case = FixtureCase::new("dump-hello", "hello.kobo");
    let output = run_kobo(["dump"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("tier=PlainOwned"));
    assert!(output.stdout.contains("tier=PlainOwned"));
}

#[test]
fn inspect_tiered_mix_fixture_shows_v03_tiers_and_map_labels() {
    let case = FixtureCase::new("inspect-tiered-mix", "tiered_mix.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("-> plain (local-only non-Copy binding)"));
    assert!(output
        .stdout
        .contains("-> rc (sequential read-only; &T likely at migration"));
    assert!(output
        .stdout
        .contains("-> rc_refcell (mutable shared, last resort)"));
    assert!(output
        .stdout
        .contains("let config = Rc::new(AppConfig::load());"));
    assert!(output
        .stdout
        .contains("let names: Rc<RefCell<Vec<String>>>"));

    let source_map =
        fs::read_to_string(case.fixture_path.with_extension("kobo.map")).expect("map should exist");
    assert!(source_map.contains("\"ownership_tier\": \"plain\""));
    assert!(source_map.contains("\"ownership_tier\": \"rc\""));
    assert!(source_map.contains("\"ownership_tier\": \"rc_refcell\""));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__tiered_mix", output.stdout);
    });
}

#[test]
fn inspect_box_large_fixture_uses_box_owned() {
    let case = FixtureCase::new("inspect-box-large", "box_large.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("-> box (single owner, heap required (size heuristic))"));
    assert!(output
        .stdout
        .contains("let plan: Box<LargePlan> = Box::new"));
}

#[test]
fn inspect_generic_shared_fixture_uses_conservative_wrapper() {
    let case = FixtureCase::new("inspect-generic-shared", "generic_shared.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("fn touch_twice<T>(mut value: Rc<RefCell<T>>)"));
    assert!(output.stdout.contains("generic T: Copy unknown"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__generic_shared", output.stdout);
    });
}

#[test]
fn inspect_borrowed_mutable_only_fixture_uses_rc_refcell() {
    let case = FixtureCase::new(
        "inspect-borrowed-mutable-only",
        "borrowed_mutable_only.kobo",
    );
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("-> rc_refcell (generic T: Copy unknown)"));
    assert!(output.stdout.contains("use_mut(&mut *value.borrow_mut());"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__borrowed_mutable_only", output.stdout);
    });
}

#[test]
fn inspect_generic_local_fixture_stays_plain() {
    let case = FixtureCase::new("inspect-generic-local", "generic_local.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("fn id<T>(x: T) -> T"));
    assert!(!output.stdout.contains("Rc<RefCell<T>>"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__generic_local", output.stdout);
    });
}

#[test]
fn inspect_dead_and_live_borrow_move_fixtures_split_correctly() {
    let dead_case = FixtureCase::new("inspect-dead-borrow-move", "dead_borrow_move.kobo");
    let dead_output = run_kobo(["inspect"], &dead_case.fixture_path);
    assert!(
        dead_output.status.success(),
        "stderr:\n{}",
        dead_output.stderr
    );
    assert!(dead_output
        .stdout
        .contains("-> plain (dead original after assignment)"));
    assert!(dead_output.stdout.contains("let z = x;"));
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__dead_borrow_move", dead_output.stdout.clone());
    });

    let live_case = FixtureCase::new("inspect-live-borrow-move", "live_borrow_at_move.kobo");
    let live_output = run_kobo(["inspect"], &live_case.fixture_path);
    assert!(
        live_output.status.success(),
        "stderr:\n{}",
        live_output.stderr
    );
    assert!(live_output
        .stdout
        .contains("-> rc (read-only shared across 2 call sites)"));
    assert!(live_output.stdout.contains("let z = x.clone();"));
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__live_borrow_at_move", live_output.stdout.clone());
    });
}

#[test]
fn inspect_conditional_mutation_fixture_uses_rc_refcell() {
    let case = FixtureCase::new("inspect-conditional-mutation", "conditional_mutation.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("-> rc_refcell (mutable shared, last resort)"));
    assert!(output.stdout.contains("conditional mutation path"));
    assert!(output.stdout.contains("mutate(&mut *x.borrow_mut());"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__conditional_mutation", output.stdout);
    });
}

#[test]
fn inspect_clone_elision_fixture_matches_snapshot() {
    let case = FixtureCase::new("inspect-clone-elision", "clone_elision.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("-> plain (dead original after assignment)"));
    assert!(output.stdout.contains("let result = builder;"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__clone_elision", output.stdout);
    });
}

#[test]
fn inspect_small_copy_alias_fixture_uses_plain_clone_fast_path() {
    let case = FixtureCase::new("inspect-small-copy-alias", "small_copy_alias.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("let y = x.clone();"));
    assert!(!output.stdout.contains("Rc::new(SmallCopy"));
    assert!(!output.stdout.contains("clone-elision skipped"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__small_copy_alias", output.stdout);
    });
}

#[test]
fn inspect_small_string_alias_fixture_uses_wrapper_fallback() {
    let case = FixtureCase::new("inspect-small-string-alias", "small_string_alias.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("let x = Rc::new(SmallText"));
    assert!(output.stdout.contains("let y = x.clone();"));
    assert!(!output.stdout.contains("clone-elision skipped"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__small_string_alias", output.stdout);
    });
}

#[test]
fn inspect_large_copy_alias_fixture_keeps_wrapper_path_for_large_values() {
    let case = FixtureCase::new("inspect-large-copy-alias", "large_copy_alias.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("let x = Rc::new(LargeCopy"));
    assert!(output.stdout.contains("let y = x.clone();"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__large_copy_alias", output.stdout);
    });
}

#[test]
fn inspect_opaque_alias_fixture_emits_skip_annotation() {
    let case = FixtureCase::new("inspect-opaque-alias", "opaque_alias.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("clone-elision skipped: field type unknown, may allocate"));
    assert!(output.stdout.contains("let x = Rc::new(MaybeAlloc"));
    assert!(output.stdout.contains("let y = x.clone();"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__opaque_alias", output.stdout);
    });
}

#[test]
fn inspect_borrow_scope_simple_fixture_shrinks_borrow_scope() {
    let case = FixtureCase::new("inspect-borrow-scope-simple", "borrow_scope_simple.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("x.borrow_mut().push(2);"));
    assert!(!output.stdout.contains("let r = &mut *x.borrow_mut();"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__borrow_scope_simple", output.stdout);
    });
}

#[test]
fn inspect_borrow_scope_conservative_fixture_emits_annotation() {
    let case = FixtureCase::new(
        "inspect-borrow-scope-conservative",
        "borrow_scope_conservative.kobo",
    );
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("borrow-scope-conservative"));
    assert!(output.stdout.contains("let r = &mut *x.borrow_mut();"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__borrow_scope_conservative", output.stdout);
    });
}

#[test]
fn inspect_generic_equiv_fixture_is_structurally_stable() {
    let case = FixtureCase::new("inspect-generic-equiv", "generic_equiv.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output
        .stdout
        .contains("fn first<T>(mut value: Rc<RefCell<T>>)"));
    assert!(output
        .stdout
        .contains("fn second<U>(mut value: Rc<RefCell<U>>)"));
    assert_eq!(output.stdout.matches("generic T: Copy unknown").count(), 2);

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__generic_equiv", output.stdout);
    });
}

#[test]
fn inspect_shadowed_binding_fixture_keeps_shadowed_x_sites_distinct() {
    let case = FixtureCase::new("inspect-shadowed-binding", "shadowed_binding.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.matches("// kobo: x @ line").count(), 2);
    assert!(output.stdout.contains("x @ line 2 -> plain"));
    assert!(output.stdout.contains("x @ line 7 -> rc_refcell"));

    let source_map =
        fs::read_to_string(case.fixture_path.with_extension("kobo.map")).expect("map should exist");
    assert_eq!(source_map.matches("\"ownership_tier\"").count(), 2);
    assert!(source_map.contains("\"ownership_tier\": \"plain\""));
    assert!(source_map.contains("\"ownership_tier\": \"rc_refcell\""));
    let rs_lines = source_map
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("\"line\": ")
                .and_then(|value| value.strip_suffix(','))
                .and_then(|value| value.parse::<usize>().ok())
        })
        .collect::<Vec<_>>();
    assert_eq!(rs_lines.len(), 2);
    assert_ne!(rs_lines[0], rs_lines[1]);

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__shadowed_binding", output.stdout);
    });
}

#[test]
fn inspect_return_escape_fixture_defers_box_to_rc_with_annotation() {
    // Known Limitation 6: ReturnedFromFunction escape escalates to RcShared (not BoxOwned)
    // because Box<T> requires rewriting the function return type (v0.4 work).
    // The annotation "return escape: Box<T> requires signature rewrite (v0.4)" makes
    // the deferral visible in kobo inspect output.
    let case = FixtureCase::new("inspect-return-escape", "return_escape.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(
        output
            .stdout
            .contains("return escape: Box<T> requires signature rewrite (v0.4)"),
        "expected return-escape annotation; got:\n{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("Box::new"),
        "return escape must not produce Box::new in v0.3; got:\n{}",
        output.stdout
    );

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__return_escape", output.stdout);
    });
}

#[test]
fn run_return_escape_fixture_executes_without_type_error() {
    let case = FixtureCase::new("run-return-escape", "return_escape.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "kobo");
}

#[test]
fn run_tiered_mix_fixture_executes_with_shared_wrappers() {
    let case = FixtureCase::new("run-tiered-mix", "tiered_mix.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("COUNT: 3"));
    assert!(output.stdout.contains("kobo"));
    assert!(output.stdout.ends_with('4'));
}

#[test]
fn run_conditional_mutation_fixture_drops_borrow_before_println() {
    let case = FixtureCase::new("run-conditional-mutation", "conditional_mutation.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "4");
}

#[test]
fn run_borrow_scope_simple_fixture_executes_without_borrow_panic() {
    let case = FixtureCase::new("run-borrow-scope-simple", "borrow_scope_simple.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "2");
}

#[test]
fn run_borrow_scope_conservative_fixture_executes_without_borrow_panic() {
    let case = FixtureCase::new(
        "run-borrow-scope-conservative",
        "borrow_scope_conservative.kobo",
    );
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "2");
}

#[test]
fn run_live_borrow_move_fixture_clones_shared_binding() {
    let case = FixtureCase::new("run-live-borrow-move", "live_borrow_at_move.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("5"));
}

#[test]
fn run_small_copy_alias_fixture_uses_plain_clone_output() {
    let case = FixtureCase::new("run-small-copy-alias", "small_copy_alias.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "3");
}

#[test]
fn run_small_string_alias_fixture_executes_with_wrapper_fallback() {
    let case = FixtureCase::new("run-small-string-alias", "small_string_alias.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "8");
}

#[test]
fn run_large_copy_alias_fixture_executes_with_large_value_wrapper() {
    let case = FixtureCase::new("run-large-copy-alias", "large_copy_alias.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "18");
}

#[test]
fn run_opaque_alias_fixture_executes_with_skip_annotation_path() {
    let case = FixtureCase::new("run-opaque-alias", "opaque_alias.kobo");
    let output = run_kobo(["run"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout.trim(), "20");
}

#[test]
fn inspect_resource_fixture_wraps_file_in_scoped_handle() {
    let case = FixtureCase::new("inspect-resource", "resource.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.contains("ScopedHandle::new"));
}

#[test]
fn fmt_copy_only_fixture_rewrites_lossless_source() {
    let workspace_root = workspace_root();
    let unique_id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = workspace_root
        .join("target-test-fixtures")
        .join(format!("fmt-copy-only-{}-{unique_id}", std::process::id()));
    fs::create_dir_all(&root).expect("temp fixture dir should be creatable");
    let fixture_path = root.join("copy_only.kobo");
    fs::write(
        &fixture_path,
        "fn main(){\nlet count=1;\nprintln!(\"{}\", count);\n}\n",
    )
    .expect("fixture should write");

    let output = run_kobo(["fmt"], &fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "stdout should stay empty");
    let formatted = fs::read_to_string(&fixture_path).expect("formatted file should exist");
    assert!(formatted.contains("fn main() {"));
    assert!(formatted.contains("let count = 1;"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn fmt_mapped_fixture_rewrites_lossless_user_source() {
    let workspace_root = workspace_root();
    let unique_id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = workspace_root
        .join("target-test-fixtures")
        .join(format!("fmt-mapped-{}-{unique_id}", std::process::id()));
    fs::create_dir_all(&root).expect("temp fixture dir should be creatable");
    let fixture_path = root.join("mapped.kobo");
    fs::write(
        &fixture_path,
        "fn main( ){\nlet names=vec![\"alice\",\"bob\"];\nprintln!(\"{:?}\",names);\n}\n",
    )
    .expect("fixture should write");

    let output = run_kobo(["fmt"], &fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "stdout should stay empty");
    let formatted = fs::read_to_string(&fixture_path).expect("formatted file should exist");
    assert!(formatted.contains("fn main() {"));
    assert!(formatted.contains("let names = vec![\"alice\", \"bob\"];"));
    assert!(formatted.contains("println!(\"{:?}\", names);"));
    assert!(!formatted.contains("kobo-fmt-"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn fmt_wrapped_fixture_keeps_compiler_wrappers_out_of_kobo_source() {
    let case = FixtureCase::new("fmt-wrapped", "hello.kobo");

    let output = run_kobo(["fmt"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    let formatted = fs::read_to_string(&case.fixture_path).expect("formatted file should exist");
    assert!(formatted.contains("let names = vec![\"alice\", \"bob\"];"));
    assert!(!formatted.contains("Rc<RefCell"));
    assert!(!formatted.contains("// kobo:"));
}

struct KoboOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_kobo<const N: usize>(args: [&str; N], fixture_path: &Path) -> KoboOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(args)
        .arg(fixture_path)
        .current_dir(workspace_root())
        .output()
        .expect("kobo command should run");

    KoboOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

// ---------------------------------------------------------------------------
// v0.4 — kobo debt / kobo perf integration tests (BUG-10)
// ---------------------------------------------------------------------------

#[test]
fn debt_summary_output_is_single_line() {
    let case = FixtureCase::new("debt-summary", "debt_report_full.kobo");
    let output = run_kobo(["debt", "--summary"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    let lines: Vec<&str> = output.stdout.lines().collect();
    assert_eq!(lines.len(), 1, "summary must be exactly one line, got: {}", output.stdout);
    assert!(output.stdout.contains("file(s)"), "summary must mention file count");
    assert!(output.stdout.contains("line(s)"), "summary must mention line count");
}

#[test]
fn debt_json_has_schema_version() {
    let case = FixtureCase::new("debt-json", "debt_report_full.kobo");
    let output = run_kobo(["debt", "--json"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(
        output.stdout.contains("\"schema_version\": 1"),
        "JSON must contain schema_version=1, got:\n{}",
        output.stdout,
    );
}

#[test]
fn perf_no_from_flag_prints_advisory() {
    let case = FixtureCase::new("perf-no-from", "hello.kobo");
    let output = run_kobo(["perf"], &case.fixture_path);

    assert!(output.status.success(), "perf without --from should succeed");
    assert!(
        output.stderr.contains("advisory"),
        "stderr must contain 'advisory', got:\n{}",
        output.stderr,
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}

// ---------------------------------------------------------------------------
// G2 — CLI mode flag tests [R6-01]
// ---------------------------------------------------------------------------

#[test]
fn test_cli_mode_run_default_script_succeeds() {
    // kobo run <file> — default Script mode — must compile and run without
    // diagnostic noise
    let case = FixtureCase::new("cli-mode-run-default", "hello.kobo");
    let output = run_kobo(["run"], &case.fixture_path);
    assert!(
        output.status.success(),
        "run without flags should succeed (script mode default)\nstderr:\n{}",
        output.stderr
    );
}

#[test]
fn test_cli_mode_run_checked_accepted() {
    // kobo run --checked <file> — must be accepted by clap (not "unexpected argument")
    // and must compile + run successfully without emitting errors
    let case = FixtureCase::new("cli-mode-run-checked", "hello.kobo");
    let output = run_kobo(["run", "--checked"], &case.fixture_path);
    assert!(
        !output.stderr.contains("unexpected argument"),
        "--checked must be accepted as a valid flag, got stderr:\n{}",
        output.stderr
    );
    assert!(
        output.status.success(),
        "run --checked should succeed for a well-formed file\nstderr:\n{}",
        output.stderr
    );
}

#[test]
fn test_cli_mode_run_strict_rejected_with_message() {
    // kobo run --strict <file> — must exit non-zero with "not yet implemented"
    let case = FixtureCase::new("cli-mode-run-strict", "hello.kobo");
    let output = run_kobo(["run", "--strict"], &case.fixture_path);
    assert!(
        !output.status.success(),
        "run --strict must exit non-zero"
    );
    assert!(
        output.stderr.contains("not yet implemented"),
        "stderr must mention 'not yet implemented', got:\n{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("--checked"),
        "rejection message must suggest --checked, got:\n{}",
        output.stderr
    );
}

#[test]
fn test_cli_mode_run_checked_and_strict_conflict() {
    // kobo run --checked --strict <file> — clap must reject the combination
    let case = FixtureCase::new("cli-mode-run-conflict", "hello.kobo");
    let output = run_kobo(["run", "--checked", "--strict"], &case.fixture_path);
    assert!(
        !output.status.success(),
        "run --checked --strict must fail due to argument conflict"
    );
}

#[test]
fn test_cli_mode_check_checked_accepted() {
    // kobo check --checked <file> — must be accepted and run analysis successfully
    let case = FixtureCase::new("cli-mode-check-checked", "hello.kobo");
    let output = run_kobo(["check", "--checked"], &case.fixture_path);
    assert!(
        !output.stderr.contains("unexpected argument"),
        "--checked must be accepted on check subcommand, got stderr:\n{}",
        output.stderr
    );
    assert!(
        output.status.success(),
        "check --checked should succeed for a well-formed file\nstderr:\n{}",
        output.stderr
    );
}

#[test]
fn test_cli_mode_inspect_checked_accepted() {
    // kobo inspect --checked <file> — must be accepted and output RS source
    let case = FixtureCase::new("cli-mode-inspect-checked", "hello.kobo");
    let output = run_kobo(["inspect", "--checked"], &case.fixture_path);
    assert!(
        !output.stderr.contains("unexpected argument"),
        "--checked must be accepted on inspect subcommand, got stderr:\n{}",
        output.stderr
    );
    assert!(
        output.status.success(),
        "inspect --checked should succeed for a well-formed file\nstderr:\n{}",
        output.stderr
    );
    // Output must still be valid RS source
    assert!(
        output.stdout.contains("fn main"),
        "inspect --checked must output RS source containing fn main, got:\n{}",
        output.stdout
    );
}
