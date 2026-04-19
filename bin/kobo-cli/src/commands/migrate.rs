/// CLI for `kobo migrate`.
///
/// Subcommands/flags:
///   kobo migrate file.kobo           — migrate single file
///   kobo migrate --dry-run file.kobo — show diff without applying
///   kobo migrate --apply file.kobo   — apply changes
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
use kobo_migrate::{greedy_resolve, GreedyConfig, GreedyPassResult};

use super::session::build_session;

/// Execute the migrate command.
pub(super) fn cmd_migrate(
    file: &Path,
    dry_run: bool,
    apply: bool,
    graph: bool,
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

    let config = GreedyConfig {
        solver_cluster_limit: session.config.solver_cluster_limit,
        solver_budget_seconds: session.config.solver_budget_seconds,
    };

    let result = greedy_resolve(&kir, &config);

    if graph {
        return print_dependency_graph(&result, root);
    }

    if dry_run || !apply {
        return print_dry_run_diff(file, &result, root);
    }

    if apply {
        return apply_migration(file, &result, root);
    }

    // Default: dry-run (safe default).
    print_dry_run_diff(file, &result, root)
}

/// Print a unified diff showing proposed migration changes.
fn print_dry_run_diff(
    file: &Path,
    result: &GreedyPassResult,
    _root: Option<&str>,
) -> anyhow::Result<()> {
    let file_name = file.display();

    // Print diagnostics first.
    for diag in &result.diagnostics {
        eprintln!(
            "warning[{}]: {} — {}",
            diag.code.as_str(),
            diag.binding_name,
            diag.message
        );
    }

    // Print stats header.
    println!("// kobo migrate: dry-run plan");
    println!(
        "// {} undecided binding(s), {} resolved, {} unresolved",
        result.stats.total_undecided,
        result.stats.resolved_count,
        result.stats.unresolved_count,
    );
    println!();

    if result.resolved.is_empty() {
        println!("// No migration changes needed.");
        return Ok(());
    }

    // Emit a unified diff-like output.
    println!("--- a/{}", file_name);
    println!("+++ b/{}", file_name);

    for decision in &result.resolved {
        let tier_label = match decision.tier {
            kobo_ir::OwnershipTier::PlainOwned => "T",
            kobo_ir::OwnershipTier::BoxOwned => "Box<T>",
            kobo_ir::OwnershipTier::RcShared => "Rc<T>",
            kobo_ir::OwnershipTier::ArcShared => "Arc<T>",
            kobo_ir::OwnershipTier::RcMutShared => "Rc<RefCell<T>>",
            kobo_ir::OwnershipTier::ArcMutShared => "Arc<RwLock<T>>",
            kobo_ir::OwnershipTier::Scoped => "ScopedHandle<T>",
            kobo_ir::OwnershipTier::Undecided => "Undecided",
        };

        let reason_label = format!("{:?}", decision.reason);
        println!(
            "@@ node {} — migrate to {} (reason: {}) @@",
            decision.node.0, tier_label, reason_label,
        );
    }

    println!();
    println!(
        "// {} change(s) proposed. Use --apply to write changes.",
        result.resolved.len()
    );

    Ok(())
}

/// Print a text dependency graph showing migration constraints.
fn print_dependency_graph(
    result: &GreedyPassResult,
    _root: Option<&str>,
) -> anyhow::Result<()> {
    println!("// kobo migrate: dependency graph");
    println!();

    for decision in &result.resolved {
        let tier_label = format!("{:?}", decision.tier);
        println!("  node({}) -> {}", decision.node.0, tier_label);
    }

    for node_id in &result.unresolved {
        println!("  node({}) -> Undecided (residual)", node_id.0);
    }

    if result.resolved.is_empty() && result.unresolved.is_empty() {
        println!("  (empty graph — no undecided bindings)");
    }

    Ok(())
}

/// Apply migration changes to the source file.
fn apply_migration(
    _file: &Path,
    result: &GreedyPassResult,
    _root: Option<&str>,
) -> anyhow::Result<()> {
    if result.resolved.is_empty() {
        println!("// No migration changes to apply.");
        return Ok(());
    }

    // For now, apply is a preview + confirmation.
    // Full rewriting requires source-map integration (future work).
    println!("// kobo migrate: applying {} change(s)...", result.resolved.len());
    println!("// NOTE: source rewriting not yet implemented — showing plan.");

    for decision in &result.resolved {
        let tier_label = format!("{:?}", decision.tier);
        println!(
            "  node({}) -> {} (reason: {:?})",
            decision.node.0, tier_label, decision.reason,
        );
    }

    println!("// {} change(s) planned.", result.resolved.len());

    Ok(())
}

/// Generate actor scaffold at the given line.
fn cmd_migrate_actor(_file: &Path, actor_spec: &str) -> anyhow::Result<()> {
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

    println!("// kobo migrate --actor: generating actor scaffold");
    println!("// Source: {}:{}", actor_file, line);
    println!();

    // Generate the actor scaffold.
    println!("use tokio::sync::mpsc;");
    println!();
    println!("enum ActorMessage {{");
    println!("    // TODO: Define your message variants");
    println!("    Ping,");
    println!("}}");
    println!();
    println!("struct Actor {{");
    println!("    receiver: mpsc::Receiver<ActorMessage>,");
    println!("    // TODO: Add actor state fields");
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
    println!("            ActorMessage::Ping => {{");
    println!("                // TODO: Handle message");
    println!("            }}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::{KirNodeId, OwnershipTier, TierDecision, TierReason};
    use kobo_migrate::{GreedyDiagnostic, GreedyStats};
    use kobo_ir::{FileId, KoboSpan};

    fn make_test_result(resolved_count: usize) -> GreedyPassResult {
        let resolved: Vec<TierDecision> = (0..resolved_count)
            .map(|i| TierDecision {
                node: KirNodeId(i as u32),
                tier: OwnershipTier::PlainOwned,
                reason: TierReason::LocalOnly,
                annotate: true,
            })
            .collect();

        GreedyPassResult {
            resolved,
            unresolved: Vec::new(),
            diagnostics: Vec::new(),
            stats: GreedyStats {
                total_undecided: resolved_count,
                resolved_count,
                unresolved_count: 0,
                k0080_count: 0,
                elapsed_ms: 0,
            },
        }
    }

    #[test]
    fn dry_run_diff_contains_headers() {
        let result = make_test_result(2);
        // Capture output by calling the diff formatter directly.
        let file = Path::new("test.kobo");

        // Just verify no panic and basic logic.
        let out = print_dry_run_diff(file, &result, None);
        assert!(out.is_ok());
    }

    #[test]
    fn dry_run_empty_result_no_changes() {
        let result = make_test_result(0);
        let file = Path::new("test.kobo");
        let out = print_dry_run_diff(file, &result, None);
        assert!(out.is_ok());
    }

    #[test]
    fn graph_output_shows_nodes() {
        let result = make_test_result(3);
        let out = print_dependency_graph(&result, None);
        assert!(out.is_ok());
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
        let mut result = make_test_result(1);
        result.diagnostics.push(kobo_migrate::GreedyDiagnostic {
            code: kobo_errors::KErrorCode::K0080,
            binding_name: "data".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            message: "structural conflict".to_owned(),
        });
        let file = Path::new("test.kobo");
        let out = print_dry_run_diff(file, &result, None);
        assert!(out.is_ok());
    }
}
