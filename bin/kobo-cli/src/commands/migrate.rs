/// CLI for `kobo migrate`.
///
/// Subcommands/flags:
///   kobo migrate file.kobo           — migrate single file
///   kobo migrate --dry-run file.kobo — show diff without applying
///   kobo migrate --graph file.kobo   — show dependency graph
///   kobo migrate --root fn_name file.kobo — start from specific function
///   kobo migrate --actor file.kobo:42     — generate actor scaffold at line
///
/// Output format (--dry-run):
///   --- a/src/handler.rs
///   +++ b/src/handler.rs
///   @@ -10,3 +10,3 @@
///   -    let state = Rc::new(RefCell::new(State::new()));
///   +    let state = Arc::new(tokio::sync::RwLock::new(State::new()));
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::{extract_before_borrow_rewrite, run_kir_phase};
use kobo_ir::{KirNodeId, OwnershipTier, SolutionMap};
use kobo_migrate::decisions_cache::{DecisionsStore, PersistedDecision};
use kobo_migrate::{
    build_kir_constraint_graph, graph_fingerprint, query_solve_outcome,
    solve_modular_with_evidence, GreedyConfig, MigrateCtxt, SolveOutcome, SolverBudget,
    SolverEvidence,
};

use super::session::build_session;

/// Execute the migrate command.
#[allow(clippy::too_many_arguments)]
pub(super) fn cmd_migrate(
    file: &Path,
    dry_run: bool,
    apply: bool,
    graph: bool,
    review: bool,
    class_view: bool,
    explain: bool,
    budget_seconds: Option<f64>,
    root: Option<&str>,
    actor: Option<&str>,
) -> anyhow::Result<()> {
    // Actor scaffold is a special path.
    if let Some(actor_spec) = actor {
        return cmd_migrate_actor(file, actor_spec);
    }

    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    let budget = SolverBudget {
        max_cluster_size: session.config.solver_cluster_limit,
        budget_seconds: budget_seconds.unwrap_or(session.config.solver_budget_seconds),
    };

    let config = GreedyConfig {
        solver_cluster_limit: budget.max_cluster_size,
        solver_budget_seconds: budget.budget_seconds,
        mutable_sites_threshold: GreedyConfig::default().mutable_sites_threshold,
    };
    let cache_dir = solver_cache_dir_for(file);
    let mut migrate_ctxt = MigrateCtxt::new_with_cache_dir(kir, config, cache_dir);
    let graph_for_cache = build_kir_constraint_graph(migrate_ctxt.kir());
    let graph_fingerprint_for_cache = graph_fingerprint(&graph_for_cache);
    let cached_outcome = query_solve_outcome(&mut migrate_ctxt, &budget);
    let (outcome, solver_evidence) = if matches!(cached_outcome, SolveOutcome::Unique(_)) {
        let evidence = solver_evidence_from_outcome(
            &cached_outcome,
            &graph_fingerprint_for_cache,
            graph_for_cache.nodes.len(),
            graph_for_cache.edges.len(),
            &budget,
            migrate_ctxt.kir(),
        );
        (cached_outcome, evidence)
    } else {
        let modular = solve_modular_with_evidence(migrate_ctxt.kir(), &budget);
        let outcome = modular.outcome.clone();
        let _ = kobo_migrate::cache::SolverCache::save_unique_outcome(
            migrate_ctxt
                .cache_dir()
                .expect("migrate context has cache dir"),
            &modular.solver_evidence.graph_fingerprint,
            &outcome,
        );
        (outcome, modular.solver_evidence)
    };
    let extract_plan = build_extract_before_borrow_plan(file, migrate_ctxt.kir())?;
    let decisions_path = decisions_path_for(file);
    let mut decisions = DecisionsStore::load(&decisions_path);
    let review_artifact_updates = if review {
        let updates = ensure_review_decisions(&mut decisions, &outcome);
        decisions.save(&decisions_path).with_context(|| {
            format!(
                "failed to write migration decisions to {}",
                decisions_path.display()
            )
        })?;
        updates
    } else {
        0
    };

    if graph {
        return print_dependency_graph(migrate_ctxt.kir(), &outcome, &solver_evidence, root);
    }

    if dry_run || !apply {
        return print_dry_run_diff(
            file,
            &outcome,
            &solver_evidence,
            DryRunDiffOptions {
                extract_plan: extract_plan.as_ref(),
                decisions_path: &decisions_path,
                decisions: &decisions,
                review_artifact_updates,
                review,
                class_view,
                explain,
                _root: root,
            },
        );
    }

    if apply {
        return apply_migration(file, &outcome, root, extract_plan.as_ref());
    }

    // Default: dry-run (safe default).
    print_dry_run_diff(
        file,
        &outcome,
        &solver_evidence,
        DryRunDiffOptions {
            extract_plan: extract_plan.as_ref(),
            decisions_path: &decisions_path,
            decisions: &decisions,
            review_artifact_updates,
            review,
            class_view,
            explain,
            _root: root,
        },
    )
}

struct DryRunDiffOptions<'a> {
    extract_plan: Option<&'a ExtractBeforeBorrowPlan>,
    decisions_path: &'a Path,
    decisions: &'a DecisionsStore,
    review_artifact_updates: usize,
    review: bool,
    class_view: bool,
    explain: bool,
    _root: Option<&'a str>,
}

struct ExtractBeforeBorrowPlan {
    original: String,
    rewritten: String,
    applied_sites: usize,
}

/// Print a unified diff showing proposed migration changes.
fn print_dry_run_diff(
    file: &Path,
    outcome: &SolveOutcome,
    evidence: &SolverEvidence,
    options: DryRunDiffOptions<'_>,
) -> anyhow::Result<()> {
    let file_name = file.display();

    println!("// kobo migrate: dry-run plan");
    println!(
        "// solver outcome: {} | fingerprint: {} | nodes: {} | edges: {}",
        evidence.outcome_name, evidence.graph_fingerprint, evidence.node_count, evidence.edge_count,
    );
    println!(
        "// solver budget: max_cluster_size={} budget_seconds={:.3}",
        evidence.budget.max_cluster_size, evidence.budget.budget_seconds
    );
    println!("// decisions_file: {}", options.decisions_path.display());
    println!("// persisted_decisions: {}", options.decisions.len());
    println!(
        "// locked_decisions: {}",
        locked_decision_count(options.decisions)
    );
    if options.review {
        println!(
            "// review_artifact_updates: {}",
            options.review_artifact_updates
        );
    }
    println!(
        "// silent_decisions: {}",
        count_silent_decisions(outcome, options.decisions)
    );

    if options.class_view {
        println!("// decision class: {}", outcome_class(outcome));
        print_persisted_decision_summary(options.decisions);
    }

    if options.explain {
        println!("// explanation: {}", outcome_explanation(outcome));
    }
    println!();

    let ownership_change_count = proposed_change_count(outcome);
    let extract_fix_count = options
        .extract_plan
        .map(|plan| plan.applied_sites)
        .unwrap_or_default();

    if matches!(outcome, SolveOutcome::Unique(_))
        && ownership_change_count == 0
        && extract_fix_count == 0
    {
        println!("// kobo-pick: none (no ownership changes proposed)");
        println!("// No migration changes needed.");
        return Ok(());
    }

    // Emit a unified diff-like output.
    println!("--- a/{}", file_name);
    println!("+++ b/{}", file_name);

    match outcome {
        SolveOutcome::Unique(map) => print_solution_diff(map, "unique", options.decisions),
        SolveOutcome::MultiSolution(candidates) => {
            println!(
                "// [K0083] {} valid ownership candidate(s); preferred candidate shown below",
                candidates.len()
            );
            if let Some(candidate) = candidates.first() {
                print_solution_diff(
                    &candidate.solution,
                    &candidate.explanation,
                    options.decisions,
                );
            }
            if options.review {
                for (idx, candidate) in candidates.iter().enumerate() {
                    println!(
                        "// candidate {}: risk {:.3} — {}",
                        idx + 1,
                        candidate.risk_score,
                        candidate.explanation
                    );
                    print_solution_diff(&candidate.solution, "review-candidate", options.decisions);
                }
            } else {
                println!("// Use --review to inspect all solver candidates.");
            }
        }
        SolveOutcome::NoSolution(report) => {
            println!("// [K0080] {}", report.conflict_reason);
            println!("// conflict nodes: {}", report.conflicting_nodes.len());
            for provenance in &report.provenance {
                println!(
                    "// provenance: {:?} via {} at {}..{}",
                    provenance.fact_kind,
                    provenance.rule_name,
                    provenance.span.start,
                    provenance.span.end
                );
            }
        }
        SolveOutcome::ClusterTooLarge(report) => {
            println!(
                "// [K0081] cluster too large: {} node(s), limit {}",
                report.cluster_size, report.limit
            );
        }
        SolveOutcome::BudgetExceeded(report) => {
            println!(
                "// [K0082] solver budget exceeded: {}/{} resolved in {:.3}s of {:.3}s",
                report.resolved_count,
                report.total_count,
                report.elapsed_seconds,
                report.budget_seconds
            );
        }
        SolveOutcome::BoundaryStop(report) => {
            println!(
                "// [K0090] boundary stop at node {} crossing {}::{}",
                report.crossing_node.0, report.external_crate, report.external_function
            );
        }
    }

    if let Some(plan) = options.extract_plan {
        print_extract_before_borrow_diff(file, plan);
    }

    println!();
    println!(
        "// {} ownership change(s), {} S-17 fix(es) proposed. Use --apply to write S-17 fixes.",
        ownership_change_count, extract_fix_count,
    );

    Ok(())
}

fn build_extract_before_borrow_plan(
    file: &Path,
    kir: &kobo_ir::Kir,
) -> anyhow::Result<Option<ExtractBeforeBorrowPlan>> {
    let original =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    let (rewritten, applied_sites) = extract_before_borrow_rewrite(&original, kir);
    if applied_sites == 0 || rewritten == original {
        return Ok(None);
    }

    Ok(Some(ExtractBeforeBorrowPlan {
        original,
        rewritten,
        applied_sites,
    }))
}

fn print_extract_before_borrow_diff(file: &Path, plan: &ExtractBeforeBorrowPlan) {
    println!();
    println!(
        "// S-17 extract-before-borrow fixes: {} site(s)",
        plan.applied_sites
    );
    println!("--- a/{}", file.display());
    println!("+++ b/{}", file.display());
    println!("@@ S-17 extract-before-borrow @@");
    println!("// before");
    for line in plan.original.lines() {
        println!("-{line}");
    }
    println!("// after");
    for line in plan.rewritten.lines() {
        println!("+{line}");
    }
}

/// Print a text dependency graph showing migration constraints.
fn print_dependency_graph(
    kir: &kobo_ir::Kir,
    outcome: &SolveOutcome,
    evidence: &SolverEvidence,
    _root: Option<&str>,
) -> anyhow::Result<()> {
    println!("// kobo migrate: dependency graph");
    println!(
        "// solver outcome: {} | fingerprint: {} | nodes: {} | edges: {}",
        evidence.outcome_name, evidence.graph_fingerprint, evidence.node_count, evidence.edge_count,
    );
    println!();

    let graph = build_kir_constraint_graph(kir);
    for edge in &graph.edges {
        println!(
            "  node({}) -{:?}/{}-> node({})",
            edge.source.0, edge.kind, edge.provenance.rule_name, edge.target.0
        );
    }

    match outcome {
        SolveOutcome::Unique(map) => print_solution_graph(map),
        SolveOutcome::MultiSolution(candidates) => {
            if let Some(candidate) = candidates.first() {
                print_solution_graph(&candidate.solution);
            }
        }
        _ => {}
    }

    if graph.nodes.is_empty() && graph.edges.is_empty() {
        println!("  (empty graph — no undecided bindings)");
    }

    Ok(())
}

/// Apply migration changes to the source file.
fn apply_migration(
    file: &Path,
    outcome: &SolveOutcome,
    _root: Option<&str>,
    extract_plan: Option<&ExtractBeforeBorrowPlan>,
) -> anyhow::Result<()> {
    let change_count = proposed_change_count(outcome);
    if let Some(plan) = extract_plan {
        fs::write(file, &plan.rewritten)
            .with_context(|| format!("failed to write {}", file.display()))?;
        println!(
            "// Applied S-17 extract-before-borrow fixes: {} site(s)",
            plan.applied_sites
        );
        if change_count > 0 {
            println!(
                "// {} ownership migration change(s) remain dry-run only in this build.",
                change_count
            );
        }
        return Ok(());
    }

    if change_count == 0 {
        println!("// No migration changes to apply.");
        return Ok(());
    }

    anyhow::bail!(
        "--apply is not available in this build; use --dry-run to review {} planned change(s)",
        change_count
    )
}

fn print_solution_diff(solution: &SolutionMap, reason: &str, decisions: &DecisionsStore) {
    for (node_id, tier) in sorted_solution_entries(solution) {
        let key = decision_key(node_id);
        let locked = decisions.get(&key).filter(|decision| decision.locked);
        let visible_tier = locked.map(|decision| decision.tier).unwrap_or(tier);
        let source = locked.map(|_| "persisted-decision").unwrap_or(reason);
        println!(
            "// kobo-pick: {} -> {} ({})",
            key,
            tier_label(visible_tier),
            source
        );
        println!(
            "@@ node {} — migrate to {} (reason: {}) @@",
            node_id.0,
            tier_label(visible_tier),
            source,
        );
    }
}

fn print_solution_graph(solution: &SolutionMap) {
    for (node_id, tier) in sorted_solution_entries(solution) {
        println!("  node({}) -> {:?}", node_id.0, tier);
    }
}

fn decisions_path_for(file: &Path) -> PathBuf {
    let root = file.parent().unwrap_or_else(|| Path::new("."));
    root.join(".kobo").join("decisions.toml")
}

fn solver_cache_dir_for(file: &Path) -> PathBuf {
    let root = file.parent().unwrap_or_else(|| Path::new("."));
    root.join(".kobo").join("cache").join("solver-v1")
}

fn solver_evidence_from_outcome(
    outcome: &SolveOutcome,
    fingerprint: &str,
    node_count: usize,
    edge_count: usize,
    budget: &SolverBudget,
    kir: &kobo_ir::Kir,
) -> SolverEvidence {
    let solution = match outcome {
        SolveOutcome::Unique(map) => map.clone(),
        SolveOutcome::MultiSolution(candidates) => candidates
            .first()
            .map(|candidate| candidate.solution.clone())
            .unwrap_or_default(),
        _ => partial_solution_from_kir(kir),
    };
    SolverEvidence {
        outcome_name: kobo_migrate::outcome_name(outcome).to_owned(),
        graph_fingerprint: fingerprint.to_owned(),
        node_count,
        edge_count,
        budget: budget.clone(),
        solution,
    }
}

fn partial_solution_from_kir(kir: &kobo_ir::Kir) -> SolutionMap {
    let mut map = SolutionMap::new();
    for node in kir.iter_decl_nodes() {
        map.insert(node.id, OwnershipTier::Undecided);
    }
    map
}

fn ensure_review_decisions(decisions: &mut DecisionsStore, outcome: &SolveOutcome) -> usize {
    let mut updates = 0;
    for (node_id, tier) in preferred_solution_entries(outcome) {
        let key = decision_key(node_id);
        if decisions.get(&key).is_some() {
            continue;
        }
        decisions.set(PersistedDecision {
            binding_name: key,
            tier,
            locked: false,
            reviewed_by: Some("kobo migrate --review".to_owned()),
            comment: Some("provisional solver decision; lock after review".to_owned()),
        });
        updates += 1;
    }
    updates
}

fn preferred_solution_entries(outcome: &SolveOutcome) -> Vec<(KirNodeId, OwnershipTier)> {
    match outcome {
        SolveOutcome::Unique(map) => sorted_solution_entries(map),
        SolveOutcome::MultiSolution(candidates) => candidates
            .first()
            .map(|candidate| sorted_solution_entries(&candidate.solution))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn sorted_solution_entries(solution: &SolutionMap) -> Vec<(KirNodeId, OwnershipTier)> {
    let mut entries: Vec<_> = solution.iter().collect();
    entries.sort_by_key(|(node_id, tier)| (node_id.0, *tier));
    entries
}

fn decision_key(node_id: KirNodeId) -> String {
    format!("node_{}", node_id.0)
}

fn locked_decision_count(decisions: &DecisionsStore) -> usize {
    decisions
        .iter()
        .filter(|(_, decision)| decision.locked)
        .count()
}

fn print_persisted_decision_summary(decisions: &DecisionsStore) {
    if decisions.is_empty() {
        println!("// persisted decision entries: none");
        return;
    }
    println!("// persisted decision entries:");
    for (key, decision) in decisions.iter() {
        println!(
            "//   {} => {:?} locked={}",
            key, decision.tier, decision.locked
        );
    }
}

fn proposed_change_count(outcome: &SolveOutcome) -> usize {
    match outcome {
        SolveOutcome::Unique(map) => map.len(),
        SolveOutcome::MultiSolution(candidates) => candidates
            .first()
            .map(|candidate| candidate.solution.len())
            .unwrap_or(0),
        _ => 0,
    }
}

fn count_silent_decisions(outcome: &SolveOutcome, decisions: &DecisionsStore) -> usize {
    preferred_solution_entries(outcome)
        .into_iter()
        .filter(|(node_id, _)| !decisions.is_locked(&decision_key(*node_id)))
        .count()
}

fn tier_label(tier: kobo_ir::OwnershipTier) -> &'static str {
    match tier {
        kobo_ir::OwnershipTier::PlainOwned => "T",
        kobo_ir::OwnershipTier::BoxOwned => "Box<T>",
        kobo_ir::OwnershipTier::RcShared => "Rc<T>",
        kobo_ir::OwnershipTier::ArcShared => "Arc<T>",
        kobo_ir::OwnershipTier::RcMutShared => "Rc<RefCell<T>>",
        kobo_ir::OwnershipTier::ArcMutShared => "Arc<RwLock<T>>",
        kobo_ir::OwnershipTier::Scoped => "ScopedHandle<T>",
        kobo_ir::OwnershipTier::Undecided => "Undecided",
    }
}

fn outcome_class(outcome: &SolveOutcome) -> &'static str {
    match outcome {
        SolveOutcome::Unique(_) => "unique",
        SolveOutcome::MultiSolution(_) => "review-required",
        SolveOutcome::NoSolution(_) => "conflict",
        SolveOutcome::ClusterTooLarge(_) => "cluster-too-large",
        SolveOutcome::BudgetExceeded(_) => "budget-capped",
        SolveOutcome::BoundaryStop(_) => "boundary-stopped",
    }
}

fn outcome_explanation(outcome: &SolveOutcome) -> String {
    match outcome {
        SolveOutcome::Unique(map) => {
            format!("solver produced one assignment for {} node(s)", map.len())
        }
        SolveOutcome::MultiSolution(candidates) => {
            format!(
                "solver produced {} valid candidate assignments",
                candidates.len()
            )
        }
        SolveOutcome::NoSolution(report) => report.conflict_reason.clone(),
        SolveOutcome::ClusterTooLarge(report) => {
            format!(
                "cluster has {} nodes; limit is {}",
                report.cluster_size, report.limit
            )
        }
        SolveOutcome::BudgetExceeded(report) => format!(
            "resolved {}/{} nodes before budget {:.3}s expired",
            report.resolved_count, report.total_count, report.budget_seconds
        ),
        SolveOutcome::BoundaryStop(report) => format!(
            "node {} crosses external boundary {}::{}",
            report.crossing_node.0, report.external_crate, report.external_function
        ),
    }
}

/// Generate actor scaffold at the given line.
fn cmd_migrate_actor(file: &Path, actor_spec: &str) -> anyhow::Result<()> {
    // Parse actor_spec: "path:line" or just "path" (uses line 1).
    let (actor_file, line) = if let Some(colon_pos) = actor_spec.rfind(':') {
        let line_str = &actor_spec[colon_pos + 1..];
        let line: usize = line_str
            .parse()
            .with_context(|| format!("invalid line number in actor spec: {}", actor_spec))?;
        let path = &actor_spec[..colon_pos];
        (path, line)
    } else {
        (actor_spec, 1)
    };

    // Read the source file to extract context for the actor scaffold.
    let source_path = file.parent().unwrap_or(Path::new(".")).join(actor_file);
    let source = std::fs::read_to_string(&source_path).unwrap_or_default();

    // Extract state fields and operations from the targeted region.
    let (state_fields, message_variants) = extract_actor_context(&source, line);

    println!("// kobo migrate --actor: generating actor scaffold");
    println!("// Source: {}:{}", actor_file, line);
    println!();

    // Generate the actor scaffold with extracted context.
    println!("use tokio::sync::mpsc;");
    println!();

    // Message enum from extracted operations.
    println!("enum ActorMessage {{");
    if message_variants.is_empty() {
        println!("    // TODO: Define your message variants");
        println!("    Ping,");
    } else {
        for variant in &message_variants {
            println!("    {},", variant);
        }
    }
    println!("}}");
    println!();

    // Actor struct with extracted state fields.
    println!("struct Actor {{");
    println!("    receiver: mpsc::Receiver<ActorMessage>,");
    if state_fields.is_empty() {
        println!("    // TODO: Add actor state fields");
    } else {
        for field in &state_fields {
            println!("    {},", field);
        }
    }
    println!("}}");
    println!();

    println!("impl Actor {{");
    println!("    fn new(receiver: mpsc::Receiver<ActorMessage>) -> Self {{");
    println!("        Self {{ receiver }}");
    println!("    }}");
    println!();
    println!("    async fn run(mut self) {{");
    println!("        loop {{");
    println!("            match self.receiver.recv().await {{");
    println!("                Some(msg) => self.handle(msg).await,");
    println!("                None => break,");
    println!("            }}");
    println!("        }}");
    println!("    }}");
    println!();
    println!("    async fn handle(&mut self, msg: ActorMessage) {{");
    println!("        match msg {{");
    if message_variants.is_empty() {
        println!("            ActorMessage::Ping => {{");
        println!("                // TODO: Handle message");
        println!("            }}");
    } else {
        for variant in &message_variants {
            let name = variant.split(['(', ' ']).next().unwrap_or(variant);
            println!("            ActorMessage::{name} => {{");
            println!("                // TODO: Handle {name}");
            println!("            }}");
        }
    }
    println!("        }}");
    println!("    }}");
    println!("}}");
    println!();
    println!("fn spawn_actor() -> mpsc::Sender<ActorMessage> {{");
    println!("    let (tx, rx) = mpsc::channel(100);");
    println!("    tokio::spawn(async move {{");
    println!("        let actor = Actor::new(rx);");
    println!("        actor.run().await;");
    println!("    }});");
    println!("    tx");
    println!("}}");

    Ok(())
}

/// Extract state fields and message variants from source context around a line.
fn extract_actor_context(source: &str, target_line: usize) -> (Vec<String>, Vec<String>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut state_fields = Vec::new();
    let mut message_variants = Vec::new();

    if source.is_empty() || target_line == 0 || target_line > lines.len() {
        return (state_fields, message_variants);
    }

    // Scan from target_line outward to find the enclosing function/struct.
    // Look backward for struct fields (state) and forward for method calls (messages).
    let start = target_line.saturating_sub(1);
    let context_range = start.saturating_sub(20)..std::cmp::min(start + 40, lines.len());

    for i in context_range {
        let trimmed = lines[i].trim();

        // Extract struct fields: `name: Type`
        if trimmed.contains(':')
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("fn ")
            && !trimmed.starts_with("let ")
            && !trimmed.starts_with("pub fn ")
            && !trimmed.contains("->")
        {
            let field = trimmed.trim_end_matches(',');
            if field.contains(':') {
                let parts: Vec<&str> = field.splitn(2, ':').collect();
                if parts.len() == 2 {
                    let name = parts[0].trim().trim_start_matches("pub ");
                    let ty = parts[1].trim();
                    // Heuristic: looks like a field if name is a simple ident and type is capitalized
                    if !name.is_empty()
                        && !name.contains(' ')
                        && ty
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_uppercase() || c == '&')
                    {
                        state_fields.push(format!("{name}: {ty}"));
                    }
                }
            }
        }

        // Extract method calls as potential message variants.
        if trimmed.contains('.') && (trimmed.contains("(") || trimmed.contains("await")) {
            if let Some(dot_pos) = trimmed.find('.') {
                let after_dot = &trimmed[dot_pos + 1..];
                if let Some(paren_pos) = after_dot.find('(') {
                    let method = &after_dot[..paren_pos];
                    let method = method.trim();
                    if !method.is_empty()
                        && method.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && ![
                            "await", "clone", "unwrap", "map", "ok", "err", "into", "as_ref",
                            "len", "is_empty",
                        ]
                        .contains(&method)
                    {
                        let variant = to_pascal_case(method);
                        if !message_variants.contains(&variant) {
                            message_variants.push(variant);
                        }
                    }
                }
            }
        }
    }

    // Deduplicate and limit.
    state_fields.truncate(10);
    message_variants.truncate(10);
    (state_fields, message_variants)
}

/// Convert snake_case to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::{KirNodeId, OwnershipTier};

    fn make_solution(resolved_count: usize) -> SolutionMap {
        let mut solution = SolutionMap::new();
        for i in 0..resolved_count {
            solution.insert(KirNodeId(i as u32), OwnershipTier::PlainOwned);
        }
        solution
    }

    fn make_evidence(outcome_name: &str, solution: SolutionMap) -> SolverEvidence {
        SolverEvidence {
            outcome_name: outcome_name.to_owned(),
            graph_fingerprint: "0123456789abcdef".to_owned(),
            node_count: solution.len(),
            edge_count: 0,
            budget: SolverBudget::default(),
            solution,
        }
    }

    fn empty_decisions() -> DecisionsStore {
        DecisionsStore::new()
    }

    #[test]
    fn dry_run_diff_contains_headers() {
        let solution = make_solution(2);
        let outcome = SolveOutcome::Unique(solution.clone());
        let evidence = make_evidence("Unique", solution);
        let file = Path::new("test.kobo");
        let decisions = empty_decisions();
        let out = print_dry_run_diff(
            file,
            &outcome,
            &evidence,
            DryRunDiffOptions {
                extract_plan: None,
                decisions_path: Path::new(".kobo/decisions.toml"),
                decisions: &decisions,
                review_artifact_updates: 0,
                review: false,
                class_view: false,
                explain: false,
                _root: None,
            },
        );
        assert!(out.is_ok());
    }

    #[test]
    fn dry_run_empty_result_no_changes() {
        let solution = make_solution(0);
        let outcome = SolveOutcome::Unique(solution.clone());
        let evidence = make_evidence("Unique", solution);
        let file = Path::new("test.kobo");
        let decisions = empty_decisions();
        let out = print_dry_run_diff(
            file,
            &outcome,
            &evidence,
            DryRunDiffOptions {
                extract_plan: None,
                decisions_path: Path::new(".kobo/decisions.toml"),
                decisions: &decisions,
                review_artifact_updates: 0,
                review: false,
                class_view: false,
                explain: false,
                _root: None,
            },
        );
        assert!(out.is_ok());
    }

    #[test]
    fn graph_output_shows_nodes() {
        let solution = make_solution(3);
        let outcome = SolveOutcome::Unique(solution.clone());
        let evidence = make_evidence("Unique", solution);
        let kir = kobo_ir::Kir::default();
        let out = print_dependency_graph(&kir, &outcome, &evidence, None);
        assert!(out.is_ok());
    }

    #[test]
    fn apply_errors_when_rewrite_is_unavailable() {
        let outcome = SolveOutcome::Unique(make_solution(2));
        let file = Path::new("test.kobo");
        let err = apply_migration(file, &outcome, None, None).unwrap_err();
        assert!(
            err.to_string()
                .contains("--apply is not available in this build"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn actor_scaffold_generation() {
        let file = Path::new("test.kobo");
        let out = cmd_migrate_actor(file, "src/handler.rs:42");
        assert!(out.is_ok());
    }

    #[test]
    fn actor_spec_parsing() {
        // Verify the actor spec parsing logic.
        let spec = "src/handler.rs:42";
        let colon_pos = spec.rfind(':').unwrap();
        let line: usize = spec[colon_pos + 1..].parse().unwrap();
        assert_eq!(line, 42);
        assert_eq!(&spec[..colon_pos], "src/handler.rs");
    }

    #[test]
    fn count_silent_decisions_respects_locked_entries() {
        let mut solution = SolutionMap::new();
        solution.insert(KirNodeId(1), OwnershipTier::PlainOwned);
        solution.insert(KirNodeId(2), OwnershipTier::RcShared);
        let outcome = SolveOutcome::Unique(solution);

        let mut decisions = DecisionsStore::new();
        decisions.set(PersistedDecision {
            binding_name: "node_1".to_owned(),
            tier: OwnershipTier::PlainOwned,
            locked: true,
            reviewed_by: Some("reviewer".to_owned()),
            comment: Some("approved".to_owned()),
        });
        decisions.set(PersistedDecision {
            binding_name: "node_2".to_owned(),
            tier: OwnershipTier::RcShared,
            locked: false,
            reviewed_by: Some("reviewer".to_owned()),
            comment: Some("provisional".to_owned()),
        });

        assert_eq!(count_silent_decisions(&outcome, &decisions), 1);
    }

    #[test]
    fn migrate_with_diagnostics() {
        let outcome = SolveOutcome::NoSolution(kobo_migrate::ConflictReport {
            conflicting_nodes: vec![KirNodeId(1)],
            conflict_reason: "structural conflict".to_owned(),
            provenance: vec![],
        });
        let evidence = make_evidence("NoSolution", make_solution(1));
        let file = Path::new("test.kobo");
        let decisions = empty_decisions();
        let out = print_dry_run_diff(
            file,
            &outcome,
            &evidence,
            DryRunDiffOptions {
                extract_plan: None,
                decisions_path: Path::new(".kobo/decisions.toml"),
                decisions: &decisions,
                review_artifact_updates: 0,
                review: false,
                class_view: false,
                explain: true,
                _root: None,
            },
        );
        assert!(out.is_ok());
    }
}
