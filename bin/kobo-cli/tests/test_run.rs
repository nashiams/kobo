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
    assert!(output.stdout.contains("Rc<RefCell<Vec<&str>>>"));
    assert!(!output.stdout.contains("Rc<RefCell<i32>>"));
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
    assert!(output.stdout.contains("tier=RcMutShared"));
    assert!(output.stdout.contains("tier=PlainOwned"));
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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}
