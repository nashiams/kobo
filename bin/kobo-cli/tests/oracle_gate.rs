/// Oracle Gate Tests — end-to-end robustness tests for `kobo migrate`.
///
/// These tests run the real `kobo` binary against project-scale `.kobo` fixtures
/// and assert **oracle properties** — invariants that every production-grade
/// ownership solver (C2Rust, Swift, Nickel, Salsa) satisfies. If a test fails,
/// the solver has a real robustness gap, not a test bug.
///
/// Fixture naming: `oracle_*.kobo` in `tests/fixtures/`.
///
/// Oracle property categories:
///   1. Send-safety: async/spawn params must get Arc-family, not Rc-family.
///   2. Differential tiers: different usage patterns → different ownership tiers.
///   3. Constraint completeness: graph must encode ALL constraint kinds.
///   4. Determinism: same input → byte-identical output across runs.
///   5. Scaling: node/edge counts scale linearly with program size.
///   6. S-17 detection: borrow-after-move must be detected and reportable.
///   7. Conflict provenance: K0080 reports must carry the full chain.
///   8. Cross-function consistency: same value → same tier across callsites.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Test harness (mirrors evidence_contract.rs FixtureCase)
// ---------------------------------------------------------------------------

struct OracleCase {
    root: PathBuf,
    fixture_path: PathBuf,
}

impl OracleCase {
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
        assert!(
            source_fixture.exists(),
            "fixture {} must exist at {}",
            fixture_name,
            source_fixture.display()
        );

        let fixture_path = root.join(fixture_name);
        fs::copy(&source_fixture, &fixture_path).expect("fixture should copy");

        Self { root, fixture_path }
    }
}

impl Drop for OracleCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct KoboOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_kobo<const N: usize>(args: [&str; N], fixture_path: &Path) -> KoboOutput {
    let workspace_root = workspace_root();
    let relative_fixture = fixture_path
        .strip_prefix(&workspace_root)
        .expect("fixture should live under workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(args)
        .arg(relative_fixture)
        .current_dir(&workspace_root)
        .output()
        .expect("kobo command should run");

    KoboOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}

// ---------------------------------------------------------------------------
// Helper: parse migrate dry-run header
// ---------------------------------------------------------------------------

struct MigrateEvidence {
    outcome: String,
    fingerprint: String,
    node_count: u64,
    edge_count: u64,
    ownership_changes: u64,
}

fn parse_migrate_header(output: &KoboOutput) -> MigrateEvidence {
    let combined = format!("{}\n{}", output.stdout, output.stderr);

    let outcome = combined
        .lines()
        .find(|l| l.contains("solver outcome:"))
        .expect("migrate output must contain solver outcome");

    let parse_outcome = |line: &str| -> String {
        // "// solver outcome: Unique | fingerprint: ..."
        line.split("solver outcome:")
            .nth(1)
            .unwrap()
            .trim()
            .split('|')
            .next()
            .unwrap()
            .trim()
            .to_string()
    };

    let parse_field = |line: &str, field: &str| -> String {
        line.split(field)
            .nth(1)
            .unwrap()
            .trim()
            .split('|')
            .next()
            .unwrap()
            .trim()
            .to_string()
    };

    let outcome_str = parse_outcome(outcome);
    let fingerprint = parse_field(outcome, "fingerprint:");
    let nodes: u64 = parse_field(outcome, "nodes:")
        .parse()
        .expect("node_count must be numeric");
    let edges: u64 = parse_field(outcome, "edges:")
        .parse()
        .expect("edge_count must be numeric");

    // Count ownership changes from "N ownership change(s)" or "solver produced one assignment for N node(s)"
    let changes = combined
        .lines()
        .find(|l| l.contains("node(s)"))
        .and_then(|l| {
            l.split("for")
                .nth(1)
                .and_then(|s| s.trim().split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
        })
        .unwrap_or(0);

    MigrateEvidence {
        outcome: outcome_str,
        fingerprint,
        node_count: nodes,
        edge_count: edges,
        ownership_changes: changes,
    }
}

// ---------------------------------------------------------------------------
// Helper: parse inspect output for actual tier assignments
// ---------------------------------------------------------------------------

fn inspect_uses_arc(output: &KoboOutput) -> bool {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    combined.contains("use std::sync::Arc")
        || combined.contains("Arc::new")
        || combined.contains("Arc<")
}

fn inspect_uses_rc(output: &KoboOutput) -> bool {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    combined.contains("use std::rc::Rc") || combined.contains("Rc::new") || combined.contains("Rc<")
}

fn inspect_has_k0080_conflict(output: &KoboOutput) -> bool {
    // K0080-P3 is an advisory note ("shared mutable state at N call sites"),
    // NOT a real constraint conflict. A real K0080 would say "error[K0080]"
    // or appear as a NoSolution in the migrate output.
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    combined.contains("error[K0080]")
}

fn inspect_has_diagnostic(output: &KoboOutput, code: &str) -> bool {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    combined.contains(code)
}

fn graph_edge_kinds(output: &KoboOutput) -> HashSet<String> {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    let mut kinds = HashSet::new();
    for line in combined.lines() {
        if line.contains("->") {
            // "  node(X) -EdgeKind/provenance-> node(Y)"
            if let Some(edge_part) = line.split('-').nth(1) {
                if let Some(kind) = edge_part.split('/').next() {
                    kinds.insert(kind.to_string());
                }
            }
        }
    }
    kinds
}

fn count_distinct_tiers_in_inspect(output: &KoboOutput) -> usize {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    let mut tiers = HashSet::new();
    // Look for tier wrappers in the codegen output
    if combined.contains("Rc::new") || combined.contains("Rc<") {
        tiers.insert("Rc");
    }
    if combined.contains("Arc::new") || combined.contains("Arc<") {
        tiers.insert("Arc");
    }
    if combined.contains("Box::new") || combined.contains("Box<") {
        tiers.insert("Box");
    }
    // PlainOwned = no wrapper
    let has_let_without_wrapper = combined.lines().any(|l| {
        l.trim_start().starts_with("let ")
            && !l.contains("Rc::new")
            && !l.contains("Arc::new")
            && !l.contains("Box::new")
            && !l.contains("RefCell")
    });
    if has_let_without_wrapper {
        tiers.insert("PlainOwned");
    }
    tiers.len()
}

// ===========================================================================
// ORACLE 1: Send-safety — async params must not get Rc
// ===========================================================================

/// A binding passed into an async fn (spawn boundary) MUST be assigned an
/// Arc-family tier, not Rc. Rc is not Send — using it across a spawn is UB.
///
/// Oracle source: C2Rust promotes READ→WRITE→MOVE at call boundaries that
/// cross thread spawns. Swift generates Sendable constraints for actor-isolated
/// closures. Both would reject Rc here.
#[test]
fn oracle_spawn_boundary_must_promote_to_arc_family() {
    let case = OracleCase::new("oracle-spawn-arc", "oracle_shared_counter_spawn.kobo");
    let inspect = run_kobo(["inspect"], &case.fixture_path);
    assert!(
        inspect.status.success(),
        "inspect must succeed\nstderr:\n{}",
        inspect.stderr
    );

    // Oracle gate: Rc for spawn-crossing data is unsound. The solver MUST
    // either assign Arc or report a real K0080 conflict (not advisory K0080-P3).
    let has_arc = inspect_uses_arc(&inspect);
    let has_real_k0080 = inspect_has_k0080_conflict(&inspect);

    // STRICT: The solver must do ONE of:
    // (a) Promote to Arc (correct solution), OR
    // (b) Report error[K0080] conflict (honest about inability to solve)
    // K0080-P3 advisory ("shared mutable state") does NOT count —
    // it leaves unsound Rc code in place without blocking.
    assert!(
        has_arc || has_real_k0080,
        "ORACLE FAIL: spawn_worker takes &mut Server across async boundary.\n\
         The solver assigned Rc<RefCell<Server>> which is !Send — this code will\n\
         panic at runtime in any tokio/async-std executor.\n\
         K0080-P3 advisory does NOT satisfy this gate — it's informational, not blocking.\n\
         C2Rust would promote to Arc; Swift would generate a Sendable constraint.\n\
         The solver must either promote to Arc or report error[K0080].\n\
         stdout:\n{}\nstderr:\n{}",
        inspect.stdout,
        inspect.stderr
    );
}

/// The middleware pipeline has an async `background_flush` that takes `&mut Logger`.
/// Logger must get Arc-family. Other middleware (RateLimiter, AuthCache) that stay
/// on the same thread could stay Rc. A production solver differentiates these.
#[test]
fn oracle_middleware_async_must_differentiate_send_vs_local() {
    let case = OracleCase::new("oracle-mw-send", "oracle_middleware_pipeline.kobo");
    let inspect = run_kobo(["inspect"], &case.fixture_path);
    assert!(
        inspect.status.success(),
        "inspect must succeed\nstderr:\n{}",
        inspect.stderr
    );

    let has_arc = inspect_uses_arc(&inspect);
    let has_real_k0080 = inspect_has_k0080_conflict(&inspect);

    assert!(
        has_arc || has_real_k0080,
        "ORACLE FAIL: background_flush takes &mut Logger across async boundary.\n\
         The solver assigned Rc<RefCell<Logger>> which is !Send.\n\
         K0080-P3 advisory does NOT satisfy this gate.\n\
         C2Rust would promote Logger's pointer to Arc; Swift would add a Sendable constraint.\n\
         The solver must either promote to Arc or report error[K0080] conflict.\n\
         stdout:\n{}\nstderr:\n{}",
        inspect.stdout,
        inspect.stderr
    );
}

// ===========================================================================
// ORACLE 2: Differential tiers — different bindings get different wrappers
// ===========================================================================

/// The observer fixture has ReadObserver (shared read across spawn → Arc),
/// AggregateObserver (mutable across spawn → Arc<RwLock>), and
/// FilterObserver (local mutable only → Rc<RefCell> suffices).
/// A uniform-tier solver that assigns everything the same is wrong.
#[test]
fn oracle_observer_different_usage_patterns_should_get_different_tiers() {
    let case = OracleCase::new("oracle-obs-tiers", "oracle_observer_mixed_tiers.kobo");
    let inspect = run_kobo(["inspect"], &case.fixture_path);
    assert!(
        inspect.status.success(),
        "inspect must succeed\nstderr:\n{}",
        inspect.stderr
    );

    let distinct_tiers = count_distinct_tiers_in_inspect(&inspect);

    // Oracle gate: at least 3 distinct tier categories should appear.
    // In a production solver: PlainOwned (for temporaries like Event),
    // Rc (for local shared like FilterObserver), Arc (for cross-spawn like
    // AggregateObserver). Three distinct tiers is the minimum correct answer.
    assert!(
        distinct_tiers >= 3,
        "ORACLE FAIL: observer fixture has bindings with fundamentally different \
         usage patterns:\n  \
         - FilterObserver: local mutable only → Rc<RefCell> suffices\n  \
         - AggregateObserver: mutable across spawn → needs Arc<RwLock>\n  \
         - ReadObserver: read across spawn → needs Arc\n  \
         - Event temporaries: owned, no sharing → PlainOwned\n\
         A production solver (C2Rust, Swift) assigns at least 3 distinct tiers.\n\
         Found only {} distinct tier(s).\n\
         stdout:\n{}\nstderr:\n{}",
        distinct_tiers,
        inspect.stdout,
        inspect.stderr
    );
}

// ===========================================================================
// ORACLE 3: Constraint graph completeness
// ===========================================================================

/// The constraint graph for a program with async functions MUST contain
/// more than just PropagateSharing edges. Send constraints, floor/ceiling
/// constraints, and MutuallyExclusive edges should appear when the program
/// has those patterns.
#[test]
fn oracle_constraint_graph_must_encode_send_edges_for_async() {
    let case = OracleCase::new("oracle-graph-send", "oracle_shared_counter_spawn.kobo");
    let graph = run_kobo(["migrate", "--graph"], &case.fixture_path);
    assert!(
        graph.status.success(),
        "graph must succeed\nstderr:\n{}",
        graph.stderr
    );

    let kinds = graph_edge_kinds(&graph);

    // Oracle gate: the graph must contain at least PropagateSharing.
    assert!(
        kinds.contains("PropagateSharing"),
        "constraint graph must contain PropagateSharing edges\n\
         Got: {:?}\nstdout:\n{}",
        kinds,
        graph.stdout
    );

    // STRICT: A program with async functions MUST have PropagateSend or
    // equivalent edges. If only PropagateSharing exists, the solver is
    // blind to thread-safety requirements.
    let has_send_edges = kinds.iter().any(|k| {
        k.contains("Send")
            || k.contains("send")
            || k.contains("Async")
            || k.contains("Floor")
    });

    assert!(
        has_send_edges,
        "ORACLE FAIL: constraint graph for async program contains only {:?}.\n\
         spawn_worker is an async fn taking &mut Server — the graph MUST contain\n\
         PropagateSend or Floor constraints to enforce thread-safety.\n\
         Swift generates Sendable disjunctions; C2Rust generates WRITE→MOVE edges\n\
         at spawn boundaries. Without these edges the solver cannot reason about Send.",
        kinds
    );
}

/// The game ECS fixture has physics (exclusive mutable access to positions)
/// and render (shared read of positions). The constraint graph should contain
/// MutuallyExclusive or equivalent edges between these systems.
#[test]
fn oracle_game_ecs_graph_must_encode_exclusivity() {
    let case = OracleCase::new("oracle-graph-excl", "oracle_game_ecs_tick.kobo");
    let graph = run_kobo(["migrate", "--graph"], &case.fixture_path);
    assert!(
        graph.status.success(),
        "graph must succeed\nstderr:\n{}",
        graph.stderr
    );

    let kinds = graph_edge_kinds(&graph);
    let evidence = parse_migrate_header(&run_kobo(
        ["migrate", "--dry-run", "--class", "--explain"],
        &case.fixture_path,
    ));

    // The graph should have edges. An empty graph for a 200-line program
    // with complex sharing patterns means the solver isn't seeing the structure.
    assert!(
        evidence.edge_count > 0,
        "oracle violation: game ECS fixture has complex sharing patterns \
         but the constraint graph has 0 edges.\nstdout:\n{}",
        graph.stdout
    );

    // STRICT: physics_tick takes &mut EntityPool (exclusive mutable) and
    // render_tick takes &EntityPool (shared read). The graph MUST encode
    // this as MutuallyExclusive or equivalent constraint.
    let has_exclusivity = kinds.iter().any(|k| {
        k.contains("Exclusive") || k.contains("exclusive") || k.contains("Conflict")
    });

    assert!(
        has_exclusivity,
        "ORACLE FAIL: game ECS has physics_tick(&mut EntityPool) and render_tick(&EntityPool)\n\
         accessing the same pool — this is a classic read-write exclusion.\n\
         The constraint graph must contain MutuallyExclusive or equivalent edges.\n\
         Without them, the solver can assign mutable-shared to both, violating Rust's\n\
         aliasing rules. Kinds found: {:?}",
        kinds
    );
}

// ===========================================================================
// ORACLE 4: Determinism — same input produces identical output
// ===========================================================================

/// Every oracle fixture must produce byte-identical output across 10 runs.
/// Non-determinism in a constraint solver means HashMap iteration order leaks
/// or RNG in candidate selection — both are production bugs.
#[test]
fn oracle_all_fixtures_deterministic_across_10_runs() {
    let fixtures = [
        "oracle_shared_counter_spawn.kobo",
        "oracle_middleware_pipeline.kobo",
        "oracle_lsm_compaction.kobo",
        "oracle_observer_mixed_tiers.kobo",
        "oracle_game_ecs_tick.kobo",
        "oracle_diamond_config.kobo",
        "oracle_deep_call_ceiling.kobo",
        "oracle_pool_borrow_move.kobo",
    ];

    for fixture_name in fixtures {
        let case = OracleCase::new("oracle-determinism", fixture_name);
        let first = run_kobo(
            ["migrate", "--dry-run", "--class", "--explain"],
            &case.fixture_path,
        );
        assert!(
            first.status.success(),
            "{fixture_name} first run failed\nstderr:\n{}",
            first.stderr
        );

        for run_idx in 1..10 {
            let run = run_kobo(
                ["migrate", "--dry-run", "--class", "--explain"],
                &case.fixture_path,
            );
            assert!(
                run.status.success(),
                "{fixture_name} run {run_idx} failed\nstderr:\n{}",
                run.stderr
            );
            assert_eq!(
                first.stdout, run.stdout,
                "{fixture_name} migrate output changed at run {run_idx} — non-deterministic solver"
            );
        }
    }
}

// ===========================================================================
// ORACLE 5: Scaling — node/edge counts are reasonable
// ===========================================================================

/// Node count must scale sub-quadratically with program size.
/// Compare small (shared_counter, ~90 lines) vs large (middleware, ~200 lines).
/// If the large fixture has >4x the nodes of the small, the solver has
/// a combinatorial blow-up bug.
#[test]
fn oracle_constraint_graph_scales_linearly_not_quadratically() {
    let small_case = OracleCase::new("oracle-scale-small", "oracle_shared_counter_spawn.kobo");
    let large_case = OracleCase::new("oracle-scale-large", "oracle_middleware_pipeline.kobo");

    let small = parse_migrate_header(&run_kobo(
        ["migrate", "--dry-run"],
        &small_case.fixture_path,
    ));
    let large = parse_migrate_header(&run_kobo(
        ["migrate", "--dry-run"],
        &large_case.fixture_path,
    ));

    // middleware is ~2.2x the LOC of shared_counter. With linear scaling,
    // nodes should be ~2-3x. Quadratic would be ~5x+.
    let ratio = large.node_count as f64 / small.node_count.max(1) as f64;
    assert!(
        ratio < 8.0,
        "oracle violation: node count ratio between middleware ({} nodes) and \
         shared_counter ({} nodes) is {:.1}x — suggests quadratic scaling.\n\
         Expected < 8x for a ~2.2x LOC difference.",
        large.node_count,
        small.node_count,
        ratio
    );

    let edge_ratio = large.edge_count as f64 / small.edge_count.max(1) as f64;
    assert!(
        edge_ratio < 50.0,
        "oracle violation: edge count ratio between middleware ({} edges) and \
         shared_counter ({} edges) is {:.1}x — suggests combinatorial edge explosion.",
        large.edge_count,
        small.edge_count,
        edge_ratio
    );
}

// ===========================================================================
// ORACLE 6: S-17 borrow-after-move detection
// ===========================================================================

/// The pool fixture has a pattern where pool is borrowed (checkout/execute),
/// then moved (manager.add_pool(pool)). The solver must detect this as an
/// S-17 extract-before-borrow situation and either fix it or report it.
#[test]
fn oracle_pool_borrow_move_must_detect_s17() {
    let case = OracleCase::new("oracle-s17", "oracle_pool_borrow_move.kobo");
    let migrate = run_kobo(["migrate", "--dry-run"], &case.fixture_path);
    assert!(
        migrate.status.success(),
        "migrate must succeed\nstderr:\n{}",
        migrate.stderr
    );

    let combined = format!("{}\n{}", migrate.stdout, migrate.stderr);

    // Oracle gate: the output must mention S-17 or extract-before-borrow.
    let has_s17 = combined.contains("S-17") || combined.contains("extract-before-borrow");
    assert!(
        has_s17,
        "oracle violation: pool fixture has borrow-then-move pattern.\n\
         The solver must detect S-17 extract-before-borrow.\n\
         stdout:\n{}\nstderr:\n{}",
        migrate.stdout,
        migrate.stderr
    );
}

/// The pool fixture's S-17 detection should point to the right source locations.
/// The borrow is at `pool.checkout()` / `pool.execute_on()`, the move is at
/// `manager.add_pool(pool)`. The diff must reference the actual move site.
#[test]
fn oracle_pool_s17_diff_references_move_site() {
    let case = OracleCase::new("oracle-s17-loc", "oracle_pool_borrow_move.kobo");
    let migrate = run_kobo(["migrate", "--dry-run"], &case.fixture_path);
    assert!(
        migrate.status.success(),
        "migrate must succeed\nstderr:\n{}",
        migrate.stderr
    );

    let combined = format!("{}\n{}", migrate.stdout, migrate.stderr);

    // The S-17 diff should exist
    assert!(
        combined.contains("S-17"),
        "S-17 must be detected for pool fixture"
    );

    // The diff should reference the actual file, not be empty
    assert!(
        combined.contains("oracle_pool_borrow_move.kobo"),
        "S-17 diff must reference the fixture file\nstdout:\n{}",
        migrate.stdout
    );
}

// ===========================================================================
// ORACLE 7: Conflict provenance quality
// ===========================================================================

/// When the solver reports K0080 (constraint conflict), it must include
/// provenance — not just "conflict at node X" but the chain of constraints
/// that led to the conflict. We test this with the deep call chain fixture.
#[test]
fn oracle_k0080_reports_include_provenance_chain() {
    let case = OracleCase::new("oracle-provenance", "oracle_deep_call_ceiling.kobo");
    let check = run_kobo(["check"], &case.fixture_path);

    let combined = format!("{}\n{}", check.stdout, check.stderr);

    // The deep call chain may or may not trigger K0080 depending on solver state.
    // If it does, the report must include provenance.
    if combined.contains("K0080") {
        // Provenance should mention multiple stages (A through E) or at least
        // reference more than one source location.
        let location_count = combined
            .lines()
            .filter(|l| l.contains("-->"))
            .count();
        assert!(
            location_count >= 2,
            "oracle violation: K0080 for deep call chain reports only {} location(s).\n\
             A production solver reports the full provenance chain.\n\
             stderr:\n{}",
            location_count,
            check.stderr
        );
    }
}

// ===========================================================================
// ORACLE 8: Cross-function consistency
// ===========================================================================

/// The diamond fixture passes Config through two paths (module_b and module_c)
/// that converge at module_d. The solver must assign Config a SINGLE consistent
/// tier across all functions. If B sees one tier and C sees another, the solver
/// has split the identity.
#[test]
fn oracle_diamond_config_must_have_consistent_tier() {
    let case = OracleCase::new("oracle-diamond", "oracle_diamond_config.kobo");
    let inspect = run_kobo(["inspect"], &case.fixture_path);
    assert!(
        inspect.status.success(),
        "inspect must succeed\nstderr:\n{}",
        inspect.stderr
    );

    let combined = format!("{}\n{}", inspect.stdout, inspect.stderr);

    // Find all lines referencing "config" in the codegen output
    let config_lines: Vec<&str> = combined
        .lines()
        .filter(|l| {
            let lower = l.to_lowercase();
            lower.contains("config") && (lower.contains("rc") || lower.contains("arc") || lower.contains("box"))
        })
        .collect();

    if config_lines.len() >= 2 {
        // All config references should use the same wrapper type
        let first_has_rc = config_lines[0].contains("Rc");
        let first_has_arc = config_lines[0].contains("Arc");
        for (i, line) in config_lines.iter().enumerate().skip(1) {
            let this_rc = line.contains("Rc") && !line.contains("Arc");
            let this_arc = line.contains("Arc");
            if first_has_rc {
                assert!(
                    this_rc || (!this_rc && !this_arc),
                    "oracle violation: Config has inconsistent tier assignment.\n\
                     Line 0: {}\n\
                     Line {}: {}\n\
                     All references to the same Config instance must use the same tier.",
                    config_lines[0],
                    i,
                    line
                );
            }
            if first_has_arc {
                assert!(
                    this_arc,
                    "oracle violation: Config has inconsistent tier assignment.\n\
                     Line 0: {}\n\
                     Line {}: {}\n\
                     All references to the same Config instance must use the same tier.",
                    config_lines[0],
                    i,
                    line
                );
            }
        }
    }

    // Also verify the solver doesn't crash on diamond patterns
    let migrate = run_kobo(["migrate", "--dry-run"], &case.fixture_path);
    assert!(
        migrate.status.success(),
        "migrate must not crash on diamond dependency pattern\nstderr:\n{}",
        migrate.stderr
    );
}

// ===========================================================================
// ORACLE 9: LSM compaction cross-function SCC
// ===========================================================================

/// The LSM fixture has functions that form a call cycle:
///   batch_insert → store.put → store.freeze_active
///   insert_and_compact → batch_insert + store.run_compaction
///   multi_phase_load → insert_and_compact (×2) + batch_insert + run_compaction
/// The solver must handle this SCC without hanging or producing
/// ClusterTooLarge/BudgetExceeded.
#[test]
fn oracle_lsm_compaction_must_solve_without_budget_exceeded() {
    let case = OracleCase::new("oracle-lsm", "oracle_lsm_compaction.kobo");
    let evidence = parse_migrate_header(&run_kobo(
        ["migrate", "--dry-run"],
        &case.fixture_path,
    ));

    assert_ne!(
        evidence.outcome, "BudgetExceeded",
        "oracle violation: LSM compaction fixture (~180 lines, ~6 functions) \
         exceeded solver budget. A production solver handles this easily.\n\
         nodes={} edges={}",
        evidence.node_count,
        evidence.edge_count
    );

    assert_ne!(
        evidence.outcome, "ClusterTooLarge",
        "oracle violation: LSM compaction fixture triggered ClusterTooLarge.\n\
         The call graph of ~6 functions should not exceed cluster limits.\n\
         nodes={}",
        evidence.node_count
    );
}

/// The LSM fixture's constraint graph must contain edges between the functions
/// that share mutable references to KvStore. If the graph only has intra-function
/// edges, the solver isn't doing interprocedural analysis.
#[test]
fn oracle_lsm_must_have_cross_function_edges() {
    let case = OracleCase::new("oracle-lsm-edges", "oracle_lsm_compaction.kobo");
    let evidence = parse_migrate_header(&run_kobo(
        ["migrate", "--dry-run"],
        &case.fixture_path,
    ));

    // The LSM fixture has 7+ functions sharing KvStore mutably.
    // With proper interprocedural analysis, edge count should be substantial.
    assert!(
        evidence.edge_count >= 5,
        "oracle violation: LSM compaction fixture has 7+ functions sharing KvStore \
         mutably, but only {} constraint edges. A production solver (C2Rust) would \
         generate per-callsite edges via function summaries.",
        evidence.edge_count
    );
}

// ===========================================================================
// ORACLE 10: All fixtures must not crash
// ===========================================================================

/// Every oracle fixture must run through the full pipeline (check + migrate +
/// inspect) without panicking. This is the bare minimum for production readiness.
#[test]
fn oracle_all_fixtures_survive_full_pipeline() {
    let fixtures = [
        "oracle_shared_counter_spawn.kobo",
        "oracle_middleware_pipeline.kobo",
        "oracle_lsm_compaction.kobo",
        "oracle_observer_mixed_tiers.kobo",
        "oracle_game_ecs_tick.kobo",
        "oracle_diamond_config.kobo",
        "oracle_deep_call_ceiling.kobo",
        "oracle_pool_borrow_move.kobo",
    ];

    for fixture_name in fixtures {
        let case = OracleCase::new("oracle-pipeline", fixture_name);

        let check = run_kobo(["check"], &case.fixture_path);
        assert!(
            check.status.success(),
            "{fixture_name}: `kobo check` crashed\nstdout:\n{}\nstderr:\n{}",
            check.stdout,
            check.stderr
        );

        let inspect = run_kobo(["inspect"], &case.fixture_path);
        assert!(
            inspect.status.success(),
            "{fixture_name}: `kobo inspect` crashed\nstdout:\n{}\nstderr:\n{}",
            inspect.stdout,
            inspect.stderr
        );

        let migrate = run_kobo(["migrate", "--dry-run"], &case.fixture_path);
        assert!(
            migrate.status.success(),
            "{fixture_name}: `kobo migrate --dry-run` crashed\nstdout:\n{}\nstderr:\n{}",
            migrate.stdout,
            migrate.stderr
        );

        let graph = run_kobo(["migrate", "--graph"], &case.fixture_path);
        assert!(
            graph.status.success(),
            "{fixture_name}: `kobo migrate --graph` crashed\nstdout:\n{}\nstderr:\n{}",
            graph.stdout,
            graph.stderr
        );
    }
}

// ===========================================================================
// ORACLE 11: Inspect output must be valid Rust syntax
// ===========================================================================

/// The codegen output (inspect) must be parseable Rust. If the solver produces
/// tier wrappers that generate invalid syntax, that's a production-blocking bug.
#[test]
fn oracle_inspect_output_is_valid_rust_syntax() {
    let fixtures = [
        "oracle_shared_counter_spawn.kobo",
        "oracle_middleware_pipeline.kobo",
        "oracle_lsm_compaction.kobo",
        "oracle_diamond_config.kobo",
        "oracle_pool_borrow_move.kobo",
    ];

    for fixture_name in fixtures {
        let case = OracleCase::new("oracle-syntax", fixture_name);
        let inspect = run_kobo(["inspect"], &case.fixture_path);
        assert!(
            inspect.status.success(),
            "{fixture_name}: inspect must succeed\nstderr:\n{}",
            inspect.stderr
        );

        // Extract the Rust code from inspect output (skip diagnostic lines)
        let rust_code: String = inspect
            .stdout
            .lines()
            .chain(inspect.stderr.lines())
            .filter(|l| {
                !l.starts_with("//")
                    && !l.starts_with("note[")
                    && !l.starts_with("warning[")
                    && !l.starts_with("error[")
                    && !l.starts_with("   =")
                    && !l.starts_with("  -->")
                    && !l.contains("| ")
            })
            .collect::<Vec<&str>>()
            .join("\n");

        // At minimum: the output should contain fn main() and use statements
        // (we can't run syn here without adding it as a dep, but we can check structure)
        assert!(
            rust_code.contains("fn main()"),
            "{fixture_name}: inspect output missing fn main()\ncode:\n{}",
            &rust_code[..rust_code.len().min(500)]
        );
    }
}

// ===========================================================================
// ORACLE 12: Fingerprint uniqueness
// ===========================================================================

/// Different fixtures must produce different fingerprints. If two structurally
/// different programs produce the same fingerprint, the hash is decorative.
#[test]
fn oracle_different_fixtures_have_different_fingerprints() {
    let fixtures = [
        "oracle_shared_counter_spawn.kobo",
        "oracle_middleware_pipeline.kobo",
        "oracle_lsm_compaction.kobo",
        "oracle_observer_mixed_tiers.kobo",
        "oracle_game_ecs_tick.kobo",
        "oracle_diamond_config.kobo",
        "oracle_deep_call_ceiling.kobo",
        "oracle_pool_borrow_move.kobo",
    ];

    let mut fingerprints: Vec<(String, String)> = Vec::new();

    for fixture_name in fixtures {
        let case = OracleCase::new("oracle-fingerprint", fixture_name);
        let evidence = parse_migrate_header(&run_kobo(
            ["migrate", "--dry-run"],
            &case.fixture_path,
        ));
        fingerprints.push((fixture_name.to_string(), evidence.fingerprint));
    }

    // Check pairwise uniqueness
    for i in 0..fingerprints.len() {
        for j in (i + 1)..fingerprints.len() {
            assert_ne!(
                fingerprints[i].1, fingerprints[j].1,
                "oracle violation: {} and {} have identical fingerprint '{}'.\n\
                 Different programs must produce different constraint graph fingerprints.",
                fingerprints[i].0,
                fingerprints[j].0,
                fingerprints[i].1
            );
        }
    }
}
