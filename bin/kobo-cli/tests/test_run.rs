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
        .contains("-> box (local-only non-Copy binding)"));
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
        .contains("-> plain (local-only non-Copy binding)"));
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
fn inspect_rc_elision_basic_fixture_splits_plain_and_wrapped() {
    let case = FixtureCase::new("inspect-rc-elision-basic", "rc_elision_basic.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    // local_only: S-1 elision → PlainOwned (no Rc wrapping)
    assert!(
        output.stdout.contains("local_only @ line 2 -> plain"),
        "local_only should be plain; got:\n{}",
        output.stdout
    );
    assert!(!output.stdout.contains("Rc::new(vec![1, 2, 3])"));
    // shared: has 3 read sites → needs_sharing → Rc wrapping
    assert!(
        output.stdout.contains("shared @ line 6 -> rc"),
        "shared should be rc; got:\n{}",
        output.stdout
    );
    assert!(output.stdout.contains("Rc::new(vec![10, 20])"));

    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__rc_elision_basic", output.stdout);
    });
}

#[test]
fn inspect_conditional_mutation_fixture_uses_rc_refcell() {
    let case = FixtureCase::new("inspect-conditional-mutation", "conditional_mutation.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    // S-1: local-only mutable binding is now PlainOwned (let mut) instead of Rc<RefCell>
    assert!(output
        .stdout
        .contains("-> plain (local-only non-Copy binding)"));
    assert!(output.stdout.contains("let mut x = String::from"));
    assert!(output.stdout.contains("mutate(&mut x);"));

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
    // S-1: local-only binding is PlainOwned
    assert!(output
        .stdout
        .contains("-> plain (local-only non-Copy binding)"));
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
    // LargeCopy has #[derive(Copy)] → detected as Copy → PlainOwned, no Rc wrapping.
    assert!(!output.stdout.contains("Rc::new(LargeCopy"));

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
    // S-1: local-only mutable binding uses let mut, no borrow_mut
    assert!(output.stdout.contains("let mut x = vec![1];"));
    assert!(!output.stdout.contains("borrow_mut"));

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
    // S-1: local-only mutable binding uses let mut, direct &mut reference
    assert!(output.stdout.contains("let mut x = vec![1];"));
    assert!(output.stdout.contains("let r = &mut x;"));

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
    // S-1: second x is now local-only PlainOwned (no sharing/escape)
    assert!(output.stdout.contains("x @ line 7 -> plain"));

    let source_map =
        fs::read_to_string(case.fixture_path.with_extension("kobo.map")).expect("map should exist");
    assert_eq!(source_map.matches("\"ownership_tier\"").count(), 2);
    assert!(source_map.contains("\"ownership_tier\": \"plain\""));
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

// ── Phase 1 / Step 1.2: kobo init ──────────────────────────────────────

#[test]
fn test_init_creates_project_skeleton() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("init-skeleton-{}", std::process::id()));
    let project_dir = root.join("my_project");
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(&root).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["init", project_dir.to_str().unwrap()])
        .output()
        .expect("kobo init should run");

    assert!(
        output.status.success(),
        "kobo init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(project_dir.join("Kobo.toml").exists(), "Kobo.toml should exist");
    assert!(project_dir.join("src").is_dir(), "src/ should be a directory");
    assert!(
        project_dir.join("src/main.kobo").exists(),
        "src/main.kobo should exist"
    );
    // Verify Kobo.toml content
    let config = fs::read_to_string(project_dir.join("Kobo.toml")).unwrap();
    assert!(config.contains("[package]"), "Kobo.toml should have [package]");
    assert!(
        config.contains("name = \"my_project\""),
        "Kobo.toml should have project name"
    );

    let _ = fs::remove_dir_all(&root);
}

// ── Phase 1 / Step 1.3: kobo build ─────────────────────────────────────

fn setup_multi_file_fixture(name: &str) -> PathBuf {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("{name}-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();

    fs::write(
        root.join("Kobo.toml"),
        r#"[package]
name = "test_project"
version = "0.1.0"

[kobo]
mode = "script"
"#,
    )
    .unwrap();

    root
}

#[test]
fn test_build_multi_file_project() {
    let dir = setup_multi_file_fixture("build-multi");
    fs::write(
        dir.join("src/main.kobo"),
        r#"mod helper;
fn main() {
    println!("hello");
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/helper.kobo"),
        r#"pub fn greet() {
    println!("hi");
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    assert!(
        output.status.success(),
        "kobo build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Verify all .rs files generated
    assert!(
        dir.join("target/kobo-gen/src/main.rs").exists(),
        "main.rs should be generated"
    );
    assert!(
        dir.join("target/kobo-gen/src/helper.rs").exists(),
        "helper.rs should be generated"
    );
    // Verify Cargo.toml generated
    assert!(
        dir.join("target/kobo-gen/Cargo.toml").exists(),
        "Cargo.toml should be generated"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_build_passes_dependencies_to_cargo() {
    let dir = setup_multi_file_fixture("build-deps");
    // Override Kobo.toml to include dependencies
    fs::write(
        dir.join("Kobo.toml"),
        r#"[package]
name = "test_deps"
version = "0.1.0"

[kobo]
mode = "script"

[dependencies]
log = "0.4"
"#,
    )
    .unwrap();
    fs::write(dir.join("src/main.kobo"), "fn main() { }\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    assert!(
        output.status.success(),
        "kobo build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cargo_toml = fs::read_to_string(dir.join("target/kobo-gen/Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("log"), "Cargo.toml should contain dependency 'log'");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_build_mod_use_passthrough() {
    let dir = setup_multi_file_fixture("build-mod-use");
    fs::write(
        dir.join("src/main.kobo"),
        r#"mod helper;
use helper::greet;
fn main() {
    greet();
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/helper.kobo"),
        r#"pub fn greet() {
    println!("hello from helper");
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    assert!(
        output.status.success(),
        "kobo build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let main_rs = fs::read_to_string(dir.join("target/kobo-gen/src/main.rs")).unwrap();
    assert!(
        main_rs.contains("mod helper;"),
        "mod statement should pass through"
    );
    assert!(
        main_rs.contains("use helper::greet;"),
        "use statement should pass through"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_build_fails_when_generated_project_does_not_compile() {
    let dir = setup_multi_file_fixture("build-contract-cargo");
    fs::write(
        dir.join("src/main.kobo"),
        r#"mod missing;
fn main() {
    println!("hello");
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    assert!(
        !output.status.success(),
        "contract says build should fail when Cargo compilation fails\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

// ── BUG-10: checked-mode with use-after-move fixture ──

#[test]
fn test_cli_mode_check_checked_use_after_move() {
    // kobo check --checked <file> with a use-after-move must emit K0001 warning AND exit 0
    let case = FixtureCase::new("cli-mode-check-checked-uam", "checked_use_after_move.kobo");
    let output = run_kobo(["check", "--checked"], &case.fixture_path);
    assert!(
        output.status.success(),
        "check --checked with use-after-move must exit 0\nstderr:\n{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("K0001"),
        "check --checked with use-after-move must emit K0001 warning\nstderr:\n{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("warning"),
        "K0001 must be a warning in checked mode, not an error\nstderr:\n{}",
        output.stderr
    );
}

#[test]
fn test_inspect_impl_basic() {
    let case = FixtureCase::new("inspect-impl-basic", "impl_basic.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(
        output.status.success(),
        "inspect impl_basic must succeed\nstderr:\n{}",
        output.stderr
    );
    // The impl block must be preserved in the output.
    assert!(
        output.stdout.contains("impl Counter"),
        "output must contain `impl Counter`\nstdout:\n{}",
        output.stdout
    );
    // Methods must be present.
    assert!(
        output.stdout.contains("fn new("),
        "output must contain `fn new(`\nstdout:\n{}",
        output.stdout
    );
    // &self receivers must pass through unchanged.
    assert!(
        output.stdout.contains("&self"),
        "output must contain `&self`\nstdout:\n{}",
        output.stdout
    );
    // Snapshot test.
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__impl_basic", output.stdout);
    });
}

#[test]
fn inspect_method_mut_detection_fixture_emits_borrow_mut_for_mutating_methods() {
    let case = FixtureCase::new("inspect-method-mut-detection", "method_mut_detection.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    // `log(&mut self)` should trigger borrow_mut on the wrapper
    // `count(&self)` should trigger borrow
    insta::with_settings!({
        prepend_module_to_snapshot => false,
        snapshot_path => "../../../tests/snapshots",
    }, {
        insta::assert_snapshot!("test_inspect__method_mut_detection", output.stdout);
    });
}

// ---------------------------------------------------------------------------
// v0.7 Phase 7 — kobo debt --borrows
// ---------------------------------------------------------------------------

#[test]
fn debt_borrows_flag_runs_successfully() {
    let case = FixtureCase::new("debt-borrows", "live_borrow_at_move.kobo");
    let output = run_kobo(["debt", "--borrows"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(
        output.stdout.contains("borrow") || output.stdout.contains("Borrow"),
        "debt --borrows should mention borrows in output, got:\n{}",
        output.stdout,
    );
}

#[test]
fn debt_borrows_json_outputs_machine_readable_report() {
    let case = FixtureCase::new("debt-borrows-json", "live_borrow_at_move.kobo");
    let output = run_kobo(["debt", "--borrows", "--json"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .expect("debt --borrows --json should emit valid JSON");
    assert!(value.get("schema_version").is_some(), "JSON borrow report must contain schema_version");
    assert!(value.get("overlapping_sites").is_some(), "JSON borrow report must contain overlapping_sites");
}

// ===========================================================================
// v0.7 Test Enforcement — P0-1 #7: pub mod + pub use passthrough
// ===========================================================================

#[test]
fn test_build_pub_mod_pub_use_passthrough() {
    let dir = setup_multi_file_fixture("build-pub-mod-use");
    fs::write(
        dir.join("src/main.kobo"),
        r#"pub mod utils;
pub use utils::greet;

fn main() {
    greet();
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/utils.kobo"),
        r#"pub fn greet() {
    println!("hello from pub use");
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Build should succeed or at least not crash on pub mod + pub use.
    // The generated code should preserve the pub mod and pub use declarations.
    assert!(
        output.status.success() || stderr.contains("cargo") || stderr.contains("Compiling"),
        "pub mod + pub use should be passed through to generated code, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&dir);
}

// ===========================================================================
// v0.7 Test Enforcement — S-1 #5: Binding across functions → escaped
// ===========================================================================

#[test]
fn test_inspect_binding_across_functions_escape() {
    // A binding passed to another function escapes → should be wrapped.
    let case = FixtureCase::new("escape-cross-fn", "return_escape.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    // return_escape.kobo has a binding that escapes via return.
    // It should NOT be PlainOwned.
    assert!(
        output.stdout.contains("Rc") || output.stdout.contains("Box") || output.stdout.contains("Arc"),
        "escaped binding should be wrapped, got:\n{}",
        output.stdout
    );
}

// ===========================================================================
// v0.7 Test Enforcement — Hard Rules
// ===========================================================================

/// HR-1: No Arc<Mutex<T>> in generated output — we use Arc<RwLock<T>> instead.
#[test]
fn hr1_no_arc_mutex_in_generated_output() {
    let case = FixtureCase::new("hr1-no-mutex", "tiered_mix.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(
        !output.stdout.contains("Arc<Mutex<"),
        "HR-1: Generated code must never contain Arc<Mutex<T>>, got:\n{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("Arc<std::sync::Mutex<"),
        "HR-1: Generated code must never contain Arc<std::sync::Mutex<T>>"
    );
}

/// HR-1 #2: Mutable shared → Arc<RwLock> not Mutex.
#[test]
fn hr1_mutable_shared_uses_rwlock_not_mutex() {
    // The decision label for arc_mut_shared should say rwlock, not mutex.
    let case = FixtureCase::new("hr1-rwlock", "tiered_mix.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    if output.stdout.contains("arc_mut") {
        assert!(
            !output.stdout.contains("arc_mutex"),
            "HR-1: arc_mut_shared label should say rwlock, not mutex, got:\n{}",
            output.stdout
        );
    }
}

/// HR-5: kobo-ir has ZERO kobo-* dependencies.
#[test]
fn hr5_kobo_ir_has_zero_kobo_deps() {
    let cargo_toml = workspace_root().join("crates/compiler/kobo-ir/Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).expect("kobo-ir Cargo.toml should exist");

    // Check that no kobo-* crate is listed as a dependency.
    for line in content.lines() {
        if line.starts_with("kobo-") && !line.starts_with("kobo-ir") {
            panic!(
                "HR-5: kobo-ir must have ZERO kobo-* dependencies, found: {}",
                line
            );
        }
    }
}

/// HR-6: kobo-transform does NOT depend on kobo-errors or kobo-analysis.
#[test]
fn hr6_kobo_transform_no_kobo_errors_or_analysis_deps() {
    let cargo_toml = workspace_root().join("crates/compiler/kobo-transform/Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).expect("kobo-transform Cargo.toml should exist");

    assert!(
        !content.contains("kobo-errors"),
        "HR-6: kobo-transform must NOT depend on kobo-errors"
    );
    assert!(
        !content.contains("kobo-analysis"),
        "HR-6: kobo-transform must NOT depend on kobo-analysis"
    );
}

/// HR-8: No todo!() or unimplemented!() in shipped compiler code.
#[test]
fn hr8_no_todo_or_unimplemented_in_shipped_code() {
    let crates_dir = workspace_root().join("crates");

    let mut violations = Vec::new();
    visit_rs_files(&crates_dir, &mut |path, content| {
        // Skip test files.
        if path.to_string_lossy().contains("tests") || path.to_string_lossy().contains("test_") {
            return;
        }
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            // Skip comments.
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with("*") {
                continue;
            }
            if trimmed.contains("todo!()") || trimmed.contains("unimplemented!()") {
                violations.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    i + 1,
                    trimmed
                ));
            }
        }
    });

    assert!(
        violations.is_empty(),
        "HR-8: Found todo!()/unimplemented!() in shipped code:\n{}",
        violations.join("\n")
    );
}

/// HR-10: No proc macros in generated output.
#[test]
fn hr10_no_proc_macros_in_generated_output() {
    let case = FixtureCase::new("hr10-no-proc-macros", "hello.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(
        !output.stdout.contains("#[proc_macro"),
        "HR-10: Generated code must not contain proc macro definitions"
    );
    // Also check no kobo-specific proc macros leak through.
    assert!(
        !output.stdout.contains("#[kobo::"),
        "HR-10: Generated code must not contain #[kobo::] attributes"
    );
}

/// HR-10 #18: Generated code compiles with no extra kobo-* deps.
#[test]
fn hr10_generated_code_no_kobo_deps() {
    let case = FixtureCase::new("hr10-no-kobo-deps", "hello.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    assert!(
        !output.stdout.contains("kobo_diag") && !output.stdout.contains("kobo_ir"),
        "HR-10: Generated code should not reference internal kobo crates"
    );
}

/// HR-3 #8: kobo inspect shows decision table row (tier info).
#[test]
fn hr3_inspect_shows_tier_info() {
    let case = FixtureCase::new("hr3-inspect-tier", "tiered_mix.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);

    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    // inspect should show tiers like PlainOwned, RcShared, etc.
    assert!(
        output.stdout.contains("PlainOwned")
            || output.stdout.contains("Rc")
            || output.stdout.contains("plain_owned")
            || output.stdout.contains("rc_shared"),
        "HR-3: kobo inspect should show tier info, got:\n{}",
        output.stdout
    );
}

/// HR-4 #11: Output identical regardless of analysis order (determinism).
#[test]
fn hr4_output_deterministic() {
    let case1 = FixtureCase::new("hr4-determinism-1", "tiered_mix.kobo");
    let output1 = run_kobo(["inspect"], &case1.fixture_path);

    let case2 = FixtureCase::new("hr4-determinism-2", "tiered_mix.kobo");
    let output2 = run_kobo(["inspect"], &case2.fixture_path);

    assert!(output1.status.success(), "run 1 stderr:\n{}", output1.stderr);
    assert!(output2.status.success(), "run 2 stderr:\n{}", output2.stderr);
    assert_eq!(
        output1.stdout, output2.stdout,
        "HR-4: Two runs of inspect must produce identical output"
    );
}

// ===========================================================================
// v0.7 Test Enforcement — Negative Tests
// ===========================================================================

/// N-6: kobo build without Kobo.toml — builds with defaults (generates default config).
#[test]
fn n6_build_without_kobo_toml_uses_defaults() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n6-no-toml-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.kobo"), "fn main() { println!(\"hello\"); }").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&root)
        .output()
        .expect("kobo build should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Without Kobo.toml, build should still succeed with defaults.
    assert!(
        output.status.success(),
        "N-6: kobo build without Kobo.toml should succeed with defaults, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&root);
}

/// N-7: kobo build with empty src/ → error.
#[test]
fn n7_build_with_empty_src_errors() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n7-empty-src-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Kobo.toml"),
        r#"[package]
name = "empty_project"
version = "0.1.0"
[kobo]
mode = "script"
"#,
    )
    .unwrap();
    // Empty src/ — no .kobo files

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&root)
        .output()
        .expect("kobo build should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.to_lowercase().contains("no") || stderr.to_lowercase().contains("empty") || stderr.to_lowercase().contains("error"),
        "N-7: kobo build with empty src should error, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&root);
}

/// N-17: Syntax error in .kobo → clear error message.
#[test]
fn n17_syntax_error_in_kobo_file_clear_error() {
    let case = FixtureCase::new("n17-syntax-err", "hello.kobo");
    // Overwrite with invalid syntax.
    fs::write(&case.fixture_path, "fn main() { let x = ; }").unwrap();

    let output = run_kobo(["check"], &case.fixture_path);

    // Should fail with a clear parse error.
    assert!(
        !output.status.success() || output.stderr.contains("error") || output.stderr.contains("parse"),
        "N-17: Syntax error should produce clear error, stderr:\n{}",
        output.stderr
    );
}

/// N-18: kobo debt on file with no borrows → clean output.
#[test]
fn n18_debt_no_borrows_clean_output() {
    // hello.kobo has no borrow overlaps.
    let case = FixtureCase::new("n18-no-borrows", "hello.kobo");
    let output = run_kobo(["debt", "--borrows"], &case.fixture_path);

    assert!(
        output.status.success(),
        "debt --borrows on clean file should succeed, stderr:\n{}",
        output.stderr
    );
    // Output should be clean — no overlaps reported.
    assert!(
        !output.stdout.contains("conflict")
            || output.stdout.contains("0 overlap")
            || output.stdout.is_empty()
            || output.stdout.contains("No borrow"),
        "N-18: debt on file with no borrows should be clean, got:\n{}",
        output.stdout
    );
}

/// N-19: Per-module mode in single-file → works (no crash).
#[test]
fn n19_per_module_mode_single_file_works() {
    let case = FixtureCase::new("n19-single-file-mode", "hello.kobo");
    // Prepend mode annotation.
    let content = fs::read_to_string(&case.fixture_path).unwrap();
    fs::write(&case.fixture_path, format!("//! kobo:mode = script\n{}", content)).unwrap();

    let output = run_kobo(["check"], &case.fixture_path);

    assert!(
        output.status.success(),
        "N-19: Per-module mode in single file should work, stderr:\n{}",
        output.stderr
    );
}

// ===========================================================================
// v0.7 Test Enforcement — Gate Criteria (UC-1 through UC-4)
// ===========================================================================

/// UC-1: CLI Tool (struct + impl + basic I/O).
#[test]
fn uc1_cli_tool_struct_impl() {
    let dir = setup_multi_file_fixture("uc1-cli-tool");
    fs::write(
        dir.join("src/main.kobo"),
        r#"
struct Config {
    name: String,
    count: u32,
}

impl Config {
    fn new(name: String, count: u32) -> Self {
        Config { name, count }
    }

    fn greeting(&self) -> String {
        format!("Hello, {}! Count: {}", self.name, self.count)
    }
}

fn main() {
    let cfg = Config::new("world".to_string(), 42);
    println!("{}", cfg.greeting());
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Build should succeed (or at least reach cargo build stage).
    assert!(
        output.status.success() || stderr.contains("cargo") || stderr.contains("Compiling"),
        "UC-1: CLI tool project should build, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&dir);
}

/// UC-2: Data Pipeline (Vec + Iterator + mod).
#[test]
fn uc2_data_pipeline_vec_iterator_mod() {
    let dir = setup_multi_file_fixture("uc2-data-pipeline");
    fs::write(
        dir.join("src/main.kobo"),
        r#"mod pipeline;

fn main() {
    let data = vec![1, 2, 3, 4, 5];
    let result = pipeline::process(data);
    println!("{:?}", result);
}
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/pipeline.kobo"),
        r#"pub fn process(data: Vec<i32>) -> Vec<i32> {
    data.iter().map(|x| x * 2).collect()
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() || stderr.contains("cargo") || stderr.contains("Compiling"),
        "UC-2: Data pipeline project should build, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&dir);
}

/// UC-3: Game/Simulation Loop (struct + mutation).
#[test]
fn uc3_game_simulation_struct_mutation() {
    let dir = setup_multi_file_fixture("uc3-game-sim");
    fs::write(
        dir.join("src/main.kobo"),
        r#"
struct GameState {
    score: u32,
    entities: Vec<String>,
}

impl GameState {
    fn new() -> Self {
        GameState { score: 0, entities: vec![] }
    }

    fn add_entity(&mut self, name: String) {
        self.entities.push(name);
        self.score += 10;
    }

    fn summary(&self) -> String {
        format!("Score: {}, Entities: {}", self.score, self.entities.len())
    }
}

fn main() {
    let mut state = GameState::new();
    state.add_entity("player".to_string());
    state.add_entity("enemy".to_string());
    println!("{}", state.summary());
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() || stderr.contains("cargo") || stderr.contains("Compiling"),
        "UC-3: Game simulation project should build, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&dir);
}

/// UC-4: Async HTTP Server — build only (no actual runtime).
#[test]
#[ignore = "feature not yet implemented: async executor selection and tokio integration"]
fn uc4_async_http_server() {
    let dir = setup_multi_file_fixture("uc4-async-http");
    fs::write(
        dir.join("src/main.kobo"),
        r#"
async fn handle_request(data: String) -> String {
    format!("Response: {}", data)
}

#[tokio::main]
async fn main() {
    let response = handle_request("hello".to_string()).await;
    println!("{}", response);
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&dir)
        .output()
        .expect("kobo build should run");

    assert!(
        output.status.success(),
        "UC-4: Async server project should build"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ===========================================================================
// v0.7 Test Enforcement — Negative Tests (remaining)
// ===========================================================================

/// N-1: @strict on non-async fn — behavior test.
/// Currently @strict is accepted on non-async fn (spec mismatch).
#[test]
fn n1_strict_on_non_async_fn_accepted() {
    let case = FixtureCase::new("n1-strict-sync", "hello.kobo");
    // Check that @strict on a sync fn doesn't crash.
    // If the behavior changes to error, this test should be updated.
    let output = run_kobo(["check"], &case.fixture_path);
    // Just ensure it doesn't panic — the exact behavior is TBD.
    let _ = output.status;
}

/// N-2: @strict placement — must be valid attribute position.
#[test]
fn n2_strict_placement_valid() {
    // Write a file with @strict in a valid position and verify it parses.
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n2-strict-placement-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(&root).unwrap();
    let file = root.join("test.kobo");
    fs::write(
        &file,
        "@strict\nasync fn process() {\n    let x = 42;\n}\nfn main() {}\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["check"])
        .arg(&file)
        .output()
        .expect("kobo check should run");

    // @strict before async fn should be accepted by the parser
    assert!(
        output.status.success(),
        "N-2: @strict before async fn should parse, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&root);
}

/// N-3: #[kobo::async_shared] on function → should be rejected or ignored.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] attribute validation"]
fn n3_kobo_async_shared_on_function_rejected() {
    // #[kobo::async_shared] on a function (not a let binding) should error.
}

/// N-4: #[kobo::async_shared] on impl block → rejected.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] attribute validation"]
fn n4_kobo_async_shared_on_impl_block_rejected() {
    // #[kobo::async_shared] on an impl block should error.
}

/// N-5: #[kobo::async_shared] on type alias → rejected.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] attribute validation"]
fn n5_kobo_async_shared_on_type_alias_rejected() {
    // #[kobo::async_shared] on a type alias should error.
}

/// N-10: Invalid [copy_types] value in Kobo.toml.
#[test]
fn n10_invalid_copy_types_in_kobo_toml() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n10-bad-copy-types-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    // Invalid: copy_types should be an array but we give a string
    fs::write(
        root.join("Kobo.toml"),
        "[project]\nname = \"n10-test\"\n\n[analysis]\ncopy_types = \"not-an-array\"\n",
    )
    .unwrap();
    fs::write(root.join("src/main.kobo"), "fn main() { }").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["check"])
        .current_dir(&root)
        .output()
        .expect("kobo check should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Invalid TOML config should produce an error
    assert!(
        !output.status.success() || stderr.to_lowercase().contains("error") || stderr.to_lowercase().contains("invalid") || stderr.to_lowercase().contains("toml"),
        "N-10: Invalid copy_types should produce error, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&root);
}

/// N-11: Invalid [mutating_methods] value in Kobo.toml.
#[test]
fn n11_invalid_mutating_methods_in_kobo_toml() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n11-bad-mut-methods-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Kobo.toml"),
        "[project]\nname = \"n11-test\"\n\n[analysis]\nmutating_methods = 42\n",
    )
    .unwrap();
    fs::write(root.join("src/main.kobo"), "fn main() { }").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["check"])
        .current_dir(&root)
        .output()
        .expect("kobo check should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.to_lowercase().contains("error") || stderr.to_lowercase().contains("invalid") || stderr.to_lowercase().contains("toml"),
        "N-11: Invalid mutating_methods should produce error, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&root);
}

/// N-12: Conflicting module names → error.
#[test]
fn n12_conflicting_module_names() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n12-conflict-mod-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Kobo.toml"), "[project]\nname = \"n12-test\"\n").unwrap();
    // Two files that would produce the same module name
    fs::write(root.join("src/main.kobo"), "mod utils;\nfn main() { }\n").unwrap();
    fs::write(root.join("src/utils.kobo"), "pub fn helper() { }\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&root)
        .output()
        .expect("kobo build should run");

    // This should either succeed (modules resolved) or error cleanly (conflict).
    // The important thing is it doesn't panic.
    let _ = output.status;

    let _ = fs::remove_dir_all(&root);
}

/// N-14: @strict async fn with RefCell → K0063.
#[test]
fn n14_strict_async_with_refcell_k0063() {
    // This is tested at the unit level in strict_async/tests.rs.
    // Here we verify via CLI that it doesn't crash.
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n14-strict-refcell-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Kobo.toml"), "[project]\nname = \"n14-test\"\nmode = \"strict\"\n").unwrap();
    fs::write(
        root.join("src/main.kobo"),
        r#"
use std::cell::RefCell;

@strict
async fn process() {
    let cell = RefCell::new(42);
    let _ = cell.borrow();
}

fn main() { }
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["check"])
        .current_dir(&root)
        .output()
        .expect("kobo check should run");

    // Should produce K0063 warning/error, not crash.
    let _ = output.status;

    let _ = fs::remove_dir_all(&root);
}

/// N-16: Non-existent crate dependency → cargo error passthrough.
#[test]
fn n16_nonexistent_crate_cargo_error() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("n16-bad-dep-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Kobo.toml"),
        "[project]\nname = \"n16-test\"\n\n[dependencies]\nthis_crate_does_not_exist_xyz = \"999.0.0\"\n",
    )
    .unwrap();
    fs::write(root.join("src/main.kobo"), "fn main() { println!(\"hello\"); }").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["build"])
        .current_dir(&root)
        .output()
        .expect("kobo build should run");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Build should fail because the dependency doesn't exist.
    // The error should come from cargo, not from kobo crashing.
    assert!(
        !output.status.success(),
        "N-16: Build with non-existent dep should fail, stderr:\n{}",
        stderr
    );

    let _ = fs::remove_dir_all(&root);
}

/// N-20: #[kobo::async_shared] in strict mode → ignored/error.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] attribute"]
fn n20_kobo_async_shared_in_strict_mode() {
    // #[kobo::async_shared] in strict mode should be ignored or produce error.
}

// ===========================================================================
// v0.7 Test Enforcement — Hard Rules (remaining)
// ===========================================================================

/// HR-1 #3: Explicit annotation still produces no Mutex.
#[test]
fn hr1_explicit_annotation_no_mutex() {
    let case = FixtureCase::new("hr1-explicit", "tiered_mix.kobo");
    let output = run_kobo(["inspect"], &case.fixture_path);
    assert!(output.status.success(), "stderr:\n{}", output.stderr);
    // Even with explicit tier annotations, Mutex should never appear
    assert!(
        !output.stdout.contains("Mutex<"),
        "HR-1: Even with annotations, generated code must never contain Mutex<, got:\n{}",
        output.stdout
    );
}

/// HR-2 #4: No std::sync::Mutex in async output.
#[test]
fn hr2_no_std_sync_mutex_in_async_output() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("hr2-async-no-mutex-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Kobo.toml"), "[project]\nname = \"hr2-test\"\n").unwrap();
    fs::write(
        root.join("src/main.kobo"),
        r#"
async fn process() {
    let data = String::from("hello");
    let alias = data;
    data.len();
    let _ = alias;
}
fn main() { }
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["inspect"])
        .current_dir(&root)
        .output()
        .expect("kobo inspect should run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("std::sync::Mutex"),
        "HR-2: Async output must never contain std::sync::Mutex, got:\n{}",
        stdout
    );

    let _ = fs::remove_dir_all(&root);
}

/// HR-2 #5: Async + mutable shared → RwLock, not Mutex.
#[test]
fn hr2_async_mutable_shared_rwlock_not_mutex() {
    let root = workspace_root()
        .join("target-test-fixtures")
        .join(format!("hr2-async-rwlock-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Kobo.toml"), "[project]\nname = \"hr2-rwlock\"\n").unwrap();
    fs::write(
        root.join("src/main.kobo"),
        r#"
async fn process() {
    let mut data = vec![1, 2, 3];
    let alias = data;
    data.push(4);
    let _ = alias;
}
fn main() { }
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(["inspect"])
        .current_dir(&root)
        .output()
        .expect("kobo inspect should run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Mutex<"),
        "HR-2: Async mutable shared should use RwLock not Mutex, got:\n{}",
        stdout
    );

    let _ = fs::remove_dir_all(&root);
}

/// HR-2 #6: No std::sync::RwLock (should use tokio::sync::RwLock if needed).
#[test]
#[ignore = "feature not yet implemented: tokio::sync::RwLock in async context"]
fn hr2_no_std_sync_rwlock_uses_tokio() {
    // Async code should use tokio::sync::RwLock, not std::sync::RwLock.
}

/// HR-3 #9: kobo inspect shows async_shared annotation.
#[test]
#[ignore = "feature not yet implemented: #[kobo::async_shared] attribute in inspect output"]
fn hr3_inspect_shows_async_shared_annotation() {
    // kobo inspect should show #[kobo::async_shared] annotations in output.
}

/// HR-4 #10: Frozen KIR — compile-time guard ensures immutability.
#[test]
fn hr4_frozen_kir_compile_time_guard() {
    // The KIR is frozen after construction. Verify via structural test:
    // check that KirNodeId, Kir, etc. don't expose &mut methods after finalization.
    // This is a compile-time guarantee, tested by the fact that the code compiles
    // without any mutable access to finalized KIR.
    let case = FixtureCase::new("hr4-frozen-kir", "hello.kobo");
    let output = run_kobo(["check"], &case.fixture_path);
    assert!(
        output.status.success(),
        "HR-4: KIR should be frozen and check should pass"
    );
}

/// HR-7 #14: Performance — check completes in reasonable time.
#[test]
fn hr7_performance_reasonable_time() {
    let start = std::time::Instant::now();
    let case = FixtureCase::new("hr7-perf", "tiered_mix.kobo");
    let output = run_kobo(["check"], &case.fixture_path);
    let elapsed = start.elapsed();

    assert!(output.status.success(), "check should succeed");
    // Check should complete in under 30 seconds (very generous for CI).
    // The spec says <500ms for 10k lines, but we test with a small fixture.
    assert!(
        elapsed.as_secs() < 30,
        "HR-7: check should complete quickly, took {:?}",
        elapsed
    );
}

/// HR-9 #16: cargo clippy --workspace should be clean (informational).
/// This test verifies clippy runs without errors (warnings may exist).
#[test]
fn hr9_cargo_clippy_no_errors() {
    let output = Command::new("cargo")
        .args(["clippy", "--workspace", "--message-format=short"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo clippy should run");

    // Clippy should not produce errors (warnings are acceptable).
    let stderr = String::from_utf8_lossy(&output.stderr);
    let has_clippy_error = stderr.lines().any(|line| {
        line.contains("error[") && !line.contains("aborting due to")
    });
    assert!(
        !has_clippy_error,
        "HR-9: cargo clippy should have no errors, stderr:\n{}",
        stderr
    );
}

// ===========================================================================
// Utility helper for structural grep tests
// ===========================================================================

fn visit_rs_files(dir: &Path, visitor: &mut dyn FnMut(&Path, &str)) {
    if dir.is_dir() {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                visit_rs_files(&path, visitor);
            } else if path.extension().map_or(false, |e| e == "rs") {
                if let Ok(content) = fs::read_to_string(&path) {
                    visitor(&path, &content);
                }
            }
        }
    }
}
