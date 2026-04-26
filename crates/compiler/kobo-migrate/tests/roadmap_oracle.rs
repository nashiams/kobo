use std::fs;
use std::path::{Path, PathBuf};

const PHASES: [&str; 15] = [
    "phase_00_pipeline_contract.md",
    "phase_01_constraint_extraction.md",
    "phase_02_cluster_detection.md",
    "phase_03_lattice_solver.md",
    "phase_04_chooser.md",
    "phase_05_budget_containment.md",
    "phase_06_solver_pipeline_wiring.md",
    "phase_07_function_summaries.md",
    "phase_08_bounded_backtracking.md",
    "phase_09_query_incrementality.md",
    "phase_10_decomposition_acceptance.md",
    "phase_11_migration_ux.md",
    "phase_12_async_hardening.md",
    "phase_13_complexity_budget.md",
    "phase_14_v08_gap_closure.md",
];

const CONTRACTS: [&str; 4] = [
    "anti_lie_verification.md",
    "evidence_contract.md",
    "solver_contract.md",
    "source_coverage.md",
];

#[test]
fn every_final_acceptance_item_has_oracle_coverage() {
    let roadmap = roadmap_root();
    let implement = read(roadmap.join("IMPLEMENT.md"));
    let gate_criteria = read(roadmap.join("acceptance/gate_criteria.md"));
    let integration_tests = read(roadmap.join("acceptance/integration_tests.md"));
    let solver_oracle =
        read(repo_root().join("crates/compiler/kobo-migrate/src/evidence_contract_tests.rs"));
    let cli_oracle = read(repo_root().join("bin/kobo-cli/tests/evidence_contract.rs"));
    let combined =
        format!("{implement}\n{gate_criteria}\n{integration_tests}\n{solver_oracle}\n{cli_oracle}");

    for number in 1..=11 {
        assert_contains(
            &implement,
            &format!("{number}."),
            "IMPLEMENT.md final acceptance must keep all 11 numbered gates",
        );
    }

    let coverage = [
        (
            "BUG-12",
            &[
                "non-`Unique` outcomes",
                "empty `SolutionMap`",
                "example_non_empty_residual_graph_cannot_earn_empty_unique_credit",
            ][..],
        ),
        (
            "ConstraintGraph",
            &[
                "real nodes and edges",
                "node_count",
                "edge_count",
                "constraint_needs_sharing",
            ],
        ),
        (
            "SolveOutcome variants",
            &[
                "all six `SolveOutcome`",
                "runtime_outcome_variants_have_distinct_contract_names",
                "BoundaryStop",
            ],
        ),
        (
            "K-code precedence",
            &[
                "K0090 wins over K0081",
                "oracle_cluster_limit_must_fire_before_claiming_solution",
                "oracle_zero_budget_must_not_claim_verified_solution",
            ],
        ),
        (
            "MultiSolution visibility",
            &[
                "MultiSolution",
                "K0083",
                "K0084",
                "multi-solution outcomes must carry executable candidates",
            ],
        ),
        (
            "determinism",
            &[
                "Determinism",
                "lattice_determinism.kobo",
                "graph_fingerprint",
            ],
        ),
        (
            "UC fixtures",
            &[
                "uc1_video_pipeline.kobo",
                "uc2_kv_store.kobo",
                "uc3_api_gateway.kobo",
                "uc4_game_server.kobo",
            ],
        ),
        (
            "async hardening",
            &[
                "async_localset_world.kobo",
                "guard_across_await_refcell.kobo",
                "select_read_exact_cancel.kobo",
            ],
        ),
        (
            "complexity",
            &[
                "codegen_no_nightly_features.kobo",
                "pin_hidden_user_view.kobo",
                "macro_single_pipeline.kobo",
            ],
        ),
        (
            "v0.8 gaps",
            &[
                "phase_14_v08_gap_closure.md",
                "S-17",
                "S-21",
                "watch_simple_rerun.kobo",
            ],
        ),
        (
            "source coverage",
            &[
                "contracts/source_coverage.md",
                "source coverage",
                "no missing IDs",
            ],
        ),
    ];

    for (label, needles) in coverage {
        for needle in needles {
            assert_contains(
                &combined,
                needle,
                &format!("final acceptance item `{label}` lacks oracle coverage for `{needle}`"),
            );
        }
    }
}

#[test]
fn every_contract_is_referenced_and_has_a_verification_surface() {
    let roadmap = roadmap_root();
    let implement = read(roadmap.join("IMPLEMENT.md"));
    let gate_criteria = read(roadmap.join("acceptance/gate_criteria.md"));
    let integration_tests = read(roadmap.join("acceptance/integration_tests.md"));
    let combined = format!("{implement}\n{gate_criteria}\n{integration_tests}");

    for contract in CONTRACTS {
        let contract_path = roadmap.join("contracts").join(contract);
        assert!(contract_path.is_file(), "missing contract {contract}");
        let contract_text = read(&contract_path);

        assert_contains(
            &combined,
            &format!("contracts/{contract}"),
            &format!("{contract} must be referenced from implementation/acceptance docs"),
        );
        assert_contains(
            &contract_text,
            "##",
            &format!("{contract} must be structured into checkable sections"),
        );
        assert!(
            contract_text.contains("test")
                || contract_text.contains("Test")
                || contract_text.contains("fail"),
            "{contract} must contain a verification/failure surface"
        );
    }

    let evidence = read(roadmap.join("contracts/evidence_contract.md"));
    for needle in [
        "Source code exists",
        "reachable from a real pipeline",
        "observed result",
        "UNKNOWN",
        "Write or identify the failing test first",
    ] {
        assert_contains(
            &evidence,
            needle,
            "evidence contract lost anti-hallucination rule",
        );
    }

    let solver = read(roadmap.join("contracts/solver_contract.md"));
    for needle in [
        "finite-lattice ownership constraint solver",
        "ProvenanceRef",
        "Precedence is strict",
        "Determinism Contract",
        "MultiSolution never collapses",
    ] {
        assert_contains(&solver, needle, "solver contract lost anti-evasion rule");
    }
}

#[test]
fn anti_lie_contract_has_one_oracle_gate_per_phase_and_required_layers() {
    let anti_lie = read(roadmap_root().join("contracts/anti_lie_verification.md"));
    let evidence = read(roadmap_root().join("contracts/evidence_contract.md"));
    let implement = read(roadmap_root().join("IMPLEMENT.md"));
    let combined = format!("{anti_lie}\n{evidence}\n{implement}");

    for phase in 0..=14 {
        let marker = format!("### Phase {phase:02}:");
        assert_contains(
            &anti_lie,
            &marker,
            "anti-lie contract must define an oracle section for every phase",
        );
    }

    for needle in [
        "Layer 1: Source-Structure Probes",
        "Layer 2: Behavioral Differential Tests",
        "Layer 3: Cross-Layer Consistency Checks",
        "assert_ne!",
        "negative test",
        "independent oracle",
        "actual `kobo` binary",
        "raw terminal output",
        "P1-P5",
    ] {
        assert_contains(
            &combined,
            needle,
            "anti-lie gate lost a hard-to-game verification requirement",
        );
    }
}

#[test]
fn executable_oracle_tests_must_include_differential_negative_and_cross_layer_checks() {
    let solver_oracle =
        read(repo_root().join("crates/compiler/kobo-migrate/src/evidence_contract_tests.rs"));
    let cli_oracle = read(repo_root().join("bin/kobo-cli/tests/evidence_contract.rs"));
    let combined = format!("{solver_oracle}\n{cli_oracle}");

    for needle in [
        "assert_ne!",
        "BudgetExceeded",
        "NoSolution",
        "run_kobo",
        "migrate",
        "inspect_source_map",
        "source_map_solver_evidence_summary",
        "brute_force_oracle",
        "oracle_seven_tier_lub_matrix_must_match_solver_lattice_contract",
    ] {
        assert_contains(
            &combined,
            needle,
            "executable oracle suite lost behavioral, negative, or cross-layer coverage",
        );
    }
}

#[test]
fn every_phase_has_test_first_acceptance_and_fixture_or_command_hooks() {
    let phases_root = roadmap_root().join("phases");

    for phase in PHASES {
        let path = phases_root.join(phase);
        assert!(path.is_file(), "missing phase file {phase}");
        let text = read(&path);

        for section in ["## Goal", "## Test First", "## Acceptance"] {
            assert_contains(&text, section, &format!("{phase} missing {section}"));
        }

        assert!(
            text.contains("fixture")
                || text.contains("Fixture")
                || text.contains("cargo test")
                || text.contains("K00")
                || text.contains("snapshot"),
            "{phase} must name a fixture, command, K-code, or snapshot verification hook"
        );
    }
}

#[test]
fn every_known_trap_has_detection_and_core_traps_have_oracle_hooks() {
    let roadmap = roadmap_root();
    let traps = read(roadmap.join("traps/known_pitfalls.md"));
    let integration_tests = read(roadmap.join("acceptance/integration_tests.md"));
    let solver_oracle =
        read(repo_root().join("crates/compiler/kobo-migrate/src/evidence_contract_tests.rs"));
    let cli_oracle = read(repo_root().join("bin/kobo-cli/tests/evidence_contract.rs"));
    let combined = format!("{traps}\n{integration_tests}\n{solver_oracle}\n{cli_oracle}");

    for trap_number in 1..=32 {
        let heading = format!("## Trap {trap_number}:");
        assert_contains(&traps, &heading, &format!("missing {heading}"));
    }

    for block in traps.split("## Trap ").skip(1) {
        assert!(
            block.contains("**Detection:**"),
            "trap block lacks Detection clause: {}",
            block.lines().next().unwrap_or("<unknown trap>")
        );
    }

    for needle in [
        "Silent Outcome Collapse",
        "MultiSolution Treated As K0080",
        "Boundary Losing Precedence",
        "Optional Provenance",
        "Non-Monotone Lattice Updates",
        "Budget Exhaustion Reported As No Solution",
        "HashMap Ordering In User Output",
        "Orphaned Public Function Counts As Implemented",
        "oracle_generated_small_graphs_must_match_bruteforce_classification",
        "inspect_must_expose_structured_solver_evidence_from_cli_path",
        "solver_graph_fingerprint_must_change_when_cli_input_changes",
    ] {
        assert_contains(&combined, needle, "core trap lacks oracle hook");
    }
}

#[test]
fn implementation_prompt_lists_required_agent_gate_commands() {
    let roadmap = roadmap_root();
    let implement = read(roadmap.join("IMPLEMENT.md"));
    let integration_tests = read(roadmap.join("acceptance/integration_tests.md"));
    let combined = format!("{implement}\n{integration_tests}");

    for needle in [
        "cargo test -p kobo-migrate evidence_contract",
        "cargo test -p kobo-cli --test evidence_contract",
        "cargo test -p kobo-migrate --test roadmap_oracle",
        "cargo nextest run",
        "cargo clippy --workspace --all-targets --all-features -- -D warnings",
        "BurntSushi.ripgrep.MSVC",
        "rg --version",
        "pre-commit run --all-files",
        "pre_commit",
    ] {
        assert_contains(
            &combined,
            needle,
            "implementation prompt lost required gate command",
        );
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root should exist")
}

fn roadmap_root() -> PathBuf {
    repo_root().join(".claude/prompt/roadmap/v0.8.1")
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn assert_contains(haystack: &str, needle: &str, message: &str) {
    assert!(haystack.contains(needle), "{message}\nmissing: {needle}");
}
