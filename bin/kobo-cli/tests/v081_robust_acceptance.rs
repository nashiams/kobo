use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

const ROBUST_FIXTURES: &[&str] = &[
    "v081_robust/robust_01_move_after_read.kobo",
    "v081_robust/robust_02_mutate_through_helper.kobo",
    "v081_robust/robust_03_shadowed_names.kobo",
    "v081_robust/robust_04_option_take_replace.kobo",
    "v081_robust/robust_05_result_match.kobo",
    "v081_robust/robust_06_enum_state_machine.kobo",
    "v081_robust/robust_07_tuple_structs.kobo",
    "v081_robust/robust_08_nested_vecs.kobo",
    "v081_robust/robust_09_generic_clone_boundary.kobo",
    "v081_robust/robust_10_generic_mutation.kobo",
    "v081_robust/robust_11_tuple_destructure.kobo",
    "v081_robust/robust_12_array_slice_read.kobo",
    "v081_robust/robust_13_hash_map_insert.kobo",
    "v081_robust/robust_14_btree_map_move.kobo",
    "v081_robust/robust_15_string_builder.kobo",
    "v081_robust/robust_16_nested_blocks_borrows.kobo",
    "v081_robust/robust_17_loop_accumulator.kobo",
    "v081_robust/robust_18_while_mutation.kobo",
    "v081_robust/robust_19_match_guard.kobo",
    "v081_robust/robust_20_struct_update.kobo",
    "v081_robust/robust_21_method_chain_read_mut.kobo",
    "v081_robust/robust_22_lifetime_return.kobo",
    "v081_robust/robust_23_async_borrow_only.kobo",
    "v081_robust/robust_24_async_mut_sequence.kobo",
    "v081_robust/robust_25_kobo_tick_attr.kobo",
    "v081_robust/robust_26_closure_capture_read.kobo",
    "v081_robust/robust_27_closure_mutate.kobo",
    "v081_robust/robust_28_recursive_function.kobo",
    "v081_robust/robust_29_binding_chain.kobo",
    "v081_robust/robust_30_many_records_stress.kobo",
];

struct FixtureCase {
    root: PathBuf,
    fixture_path: PathBuf,
}

impl FixtureCase {
    fn new(fixture_name: &str) -> Self {
        let workspace_root = workspace_root();
        let unique_id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = workspace_root
            .join("target-test-fixtures")
            .join(format!("v081-robust-{}-{unique_id}", std::process::id()));

        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        fs::create_dir_all(&root).expect("fixture temp root should be creatable");

        let source_fixture = workspace_root
            .join("tests")
            .join("fixtures")
            .join(fixture_name);
        let fixture_path = root.join(fixture_name);
        fs::create_dir_all(
            fixture_path
                .parent()
                .expect("fixture path should have parent directory"),
        )
        .expect("fixture temp subdirectory should be creatable");
        fs::copy(&source_fixture, &fixture_path).expect("robust fixture should copy");

        Self { root, fixture_path }
    }

    fn cargo_output_dir(&self) -> PathBuf {
        self.root.join("generated-cargo")
    }
}

impl Drop for FixtureCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct CmdOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

#[test]
fn v081_robust_fixtures_migrate_inspect_and_compile() {
    assert!(
        ROBUST_FIXTURES.len() >= 30,
        "robust fixture suite must keep at least 30 real .kobo cases"
    );

    let mut fixtures_with_mappings = 0;

    for fixture_name in ROBUST_FIXTURES {
        let case = FixtureCase::new(fixture_name);

        let first = run_kobo(
            ["migrate", "--dry-run", "--class", "--explain"],
            &case.fixture_path,
        );
        assert_success(&first, fixture_name, "first migrate dry-run");
        assert_migrate_contract(&first, fixture_name);

        let second = run_kobo(
            ["migrate", "--dry-run", "--class", "--explain"],
            &case.fixture_path,
        );
        assert_success(&second, fixture_name, "second migrate dry-run");
        assert_eq!(
            first.stdout, second.stdout,
            "{fixture_name} migrate output must remain deterministic across repeated runs"
        );

        let cargo_dir = case.cargo_output_dir();
        let cargo_dir_string = cargo_dir.to_string_lossy().into_owned();
        let relative_fixture = relative_to_workspace(&case.fixture_path);
        let relative_fixture_string = relative_fixture.to_string_lossy().into_owned();
        let inspect = run_kobo_with_args(&[
            "inspect",
            "--clean",
            "--cargo",
            &cargo_dir_string,
            &relative_fixture_string,
        ]);
        assert_success(&inspect, fixture_name, "inspect --cargo");

        let source_map = case.fixture_path.with_extension("kobo.map");
        assert!(
            source_map.exists(),
            "{fixture_name} inspect must write a source map for solver provenance"
        );
        if assert_solver_evidence(&source_map, fixture_name) {
            fixtures_with_mappings += 1;
        }

        let generated_main = case.cargo_output_dir().join("src").join("main.rs");
        assert!(
            generated_main.exists(),
            "{fixture_name} inspect --cargo must generate src/main.rs"
        );
        assert_generated_source_is_not_placeholder(&generated_main, fixture_name);

        let cargo_check = run_cargo_check(&case.cargo_output_dir());
        assert_success(&cargo_check, fixture_name, "generated cargo check");
    }

    assert!(
        fixtures_with_mappings >= 20,
        "robust suite should keep broad source-map provenance coverage; got {fixtures_with_mappings} mapped fixtures"
    );
}

fn assert_migrate_contract(output: &CmdOutput, fixture_name: &str) {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    assert!(
        combined.contains("solver outcome:"),
        "{fixture_name} must expose solver outcome\n{combined}"
    );
    assert!(
        combined.contains("graph fingerprint:"),
        "{fixture_name} must expose graph fingerprint\n{combined}"
    );
    assert!(
        combined.contains("solver budget:"),
        "{fixture_name} must expose solver budget\n{combined}"
    );
    assert!(
        combined.contains("decision class:"),
        "{fixture_name} must expose decision class\n{combined}"
    );
    assert!(
        combined.contains("silent_decisions: 0"),
        "{fixture_name} must not make silent ownership decisions\n{combined}"
    );
    assert!(
        !combined.contains("[K0080]"),
        "{fixture_name} must not carry unresolved ownership conflicts\n{combined}"
    );
    assert!(
        !combined.contains("[K0081]"),
        "{fixture_name} must fit within the production solver cap\n{combined}"
    );
}

fn assert_solver_evidence(source_map_path: &Path, fixture_name: &str) -> bool {
    let source_map = fs::read_to_string(source_map_path).expect("source map should read");
    let parsed: Value = serde_json::from_str(&source_map).unwrap_or_else(|error| {
        panic!("{fixture_name} source map must be valid JSON: {error}\n{source_map}")
    });

    let evidence = parsed
        .get("solver_evidence")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{fixture_name} must include top-level solver_evidence"));
    let fingerprint = evidence
        .get("graph_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{fixture_name} must include graph_fingerprint"));
    assert!(
        fingerprint.len() >= 16,
        "{fixture_name} graph fingerprint must be non-decorative: {fingerprint}"
    );
    let budget = evidence
        .get("budget")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{fixture_name} must include solver budget evidence"));
    assert_eq!(
        budget.get("max_cluster_size").and_then(Value::as_u64),
        Some(2048),
        "{fixture_name} source map must record the v0.8.1 production cluster cap"
    );

    let mappings = parsed
        .get("x_kobo_mappings")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{fixture_name} must include mapping evidence"));
    !mappings.is_empty()
}

fn assert_generated_source_is_not_placeholder(path: &Path, fixture_name: &str) {
    let source = fs::read_to_string(path).expect("generated source should read");
    for forbidden in [
        "TODO",
        "todo!",
        "unimplemented!",
        "placeholder",
        "half-generated",
    ] {
        assert!(
            !source.contains(forbidden),
            "{fixture_name} generated source must not contain {forbidden}\n{source}"
        );
    }
}

fn assert_success(output: &CmdOutput, fixture_name: &str, step: &str) {
    assert!(
        output.status.success(),
        "{fixture_name} {step} failed\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

fn run_kobo<const N: usize>(args: [&str; N], fixture_path: &Path) -> CmdOutput {
    let relative_fixture = relative_to_workspace(fixture_path);
    let relative_fixture_string = relative_fixture.to_string_lossy().into_owned();
    let mut command_args: Vec<&str> = args.to_vec();
    command_args.push(&relative_fixture_string);
    run_kobo_with_args(&command_args)
}

fn run_kobo_with_args(args: &[&str]) -> CmdOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("kobo command should run");

    CmdOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn run_cargo_check(project_dir: &Path) -> CmdOutput {
    let output = Command::new("cargo")
        .arg("check")
        .arg("--manifest-path")
        .arg(project_dir.join("Cargo.toml"))
        .current_dir(workspace_root())
        .output()
        .expect("cargo check should run");

    CmdOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn relative_to_workspace(path: &Path) -> PathBuf {
    path.strip_prefix(workspace_root())
        .expect("path should live under workspace root")
        .to_path_buf()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}
