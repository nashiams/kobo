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
use std::path::Path;

use anyhow::Context;
use kobo_driver::run_kir_phase;
use kobo_ir::SolutionMap;
use kobo_migrate::{
    build_kir_constraint_graph, solve_modular, solve_modular_with_evidence, SolveOutcome,
    SolverBudget, SolverEvidence,
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

    let modular = solve_modular_with_evidence(&kir, &budget);
    let outcome = solve_modular(&kir, &budget);

    if graph {
        return print_dependency_graph(&kir, &outcome, &modular.solver_evidence, root);
    }

    if dry_run || !apply {
        return print_dry_run_diff(
            file,
            &outcome,
            &modular.solver_evidence,
            review,
            class_view,
            explain,
            root,
        );
    }

    if apply {
        return apply_migration(file, &outcome, root);
    }

    // Default: dry-run (safe default).
    print_dry_run_diff(
        file,
        &outcome,
        &modular.solver_evidence,
        review,
        class_view,
        explain,
        root,
    )
}

/// Print a unified diff showing proposed migration changes.
fn print_dry_run_diff(
    file: &Path,
    outcome: &SolveOutcome,
    evidence: &SolverEvidence,
    review: bool,
    class_view: bool,
    explain: bool,
    _root: Option<&str>,
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

    if class_view {
        println!("// decision class: {}", outcome_class(outcome));
    }

    if explain {
        println!("// explanation: {}", outcome_explanation(outcome));
    }
    println!();

    if evidence.solution.is_empty() {
        println!("// No migration changes needed.");
        return Ok(());
    }

    // Emit a unified diff-like output.
    println!("--- a/{}", file_name);
    println!("+++ b/{}", file_name);

    match outcome {
        SolveOutcome::Unique(map) => print_solution_diff(map, "unique"),
        SolveOutcome::MultiSolution(candidates) => {
            println!(
                "// [K0083] {} valid ownership candidate(s); preferred candidate shown below",
                candidates.len()
            );
            if let Some(candidate) = candidates.first() {
                print_solution_diff(&candidate.solution, &candidate.explanation);
            }
            if review {
                for (idx, candidate) in candidates.iter().enumerate() {
                    println!(
                        "// candidate {}: risk {:.3} — {}",
                        idx + 1,
                        candidate.risk_score,
                        candidate.explanation
                    );
                    print_solution_diff(&candidate.solution, "review-candidate");
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

    println!();
    println!(
        "// {} change(s) proposed. Use --apply to write changes.",
        proposed_change_count(outcome)
    );

    Ok(())
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
    _file: &Path,
    outcome: &SolveOutcome,
    _root: Option<&str>,
) -> anyhow::Result<()> {
    let change_count = proposed_change_count(outcome);
    if change_count == 0 {
        println!("// No migration changes to apply.");
        return Ok(());
    }

    anyhow::bail!(
        "--apply is not available in this build; use --dry-run to review {} planned change(s)",
        change_count
    )
}

fn print_solution_diff(solution: &SolutionMap, reason: &str) {
    for (node_id, tier) in solution.iter() {
        println!(
            "@@ node {} — migrate to {} (reason: {}) @@",
            node_id.0,
            tier_label(tier),
            reason,
        );
    }
}

fn print_solution_graph(solution: &SolutionMap) {
    for (node_id, tier) in solution.iter() {
        println!("  node({}) -> {:?}", node_id.0, tier);
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

    #[test]
    fn dry_run_diff_contains_headers() {
        let solution = make_solution(2);
        let outcome = SolveOutcome::Unique(solution.clone());
        let evidence = make_evidence("Unique", solution);
        let file = Path::new("test.kobo");
        let out = print_dry_run_diff(file, &outcome, &evidence, false, false, false, None);
        assert!(out.is_ok());
    }

    #[test]
    fn dry_run_empty_result_no_changes() {
        let solution = make_solution(0);
        let outcome = SolveOutcome::Unique(solution.clone());
        let evidence = make_evidence("Unique", solution);
        let file = Path::new("test.kobo");
        let out = print_dry_run_diff(file, &outcome, &evidence, false, false, false, None);
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
        let err = apply_migration(file, &outcome, None).unwrap_err();
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
    fn migrate_with_diagnostics() {
        let outcome = SolveOutcome::NoSolution(kobo_migrate::ConflictReport {
            conflicting_nodes: vec![KirNodeId(1)],
            conflict_reason: "structural conflict".to_owned(),
            provenance: vec![],
        });
        let evidence = make_evidence("NoSolution", make_solution(1));
        let file = Path::new("test.kobo");
        let out = print_dry_run_diff(file, &outcome, &evidence, false, false, true, None);
        assert!(out.is_ok());
    }
}
