use kobo_ir::FileSet;

use crate::liveness::{BindingTable, LivenessState};
use crate::AnalysisFacts;

pub fn dump_facts(facts: &AnalysisFacts, file_set: &FileSet) {
    if !analysis_debug_enabled() {
        return;
    }

    eprintln!("     [kobo-analysis debug]");
    eprintln!("MoveFacts ({}):", facts.moves.len());
    for move_fact in &facts.moves {
        let move_site = render_span(file_set, move_fact.move_site);
        let later_use = render_span(file_set, move_fact.later_use);
        eprintln!(
            "  move  {}  {}  later_use: {}",
            move_fact.binding.0, move_site, later_use
        );
    }

    eprintln!("BorrowFacts ({}):", facts.borrows.len());
    for borrow_fact in &facts.borrows {
        let borrow_site = render_span(file_set, borrow_fact.borrow_site);
        let conflict_site = render_span(file_set, borrow_fact.conflict_site);
        eprintln!(
            "  borrow  {}  borrow_site: {}  conflict: {}  kind: {:?}",
            borrow_fact.binding.0, borrow_site, conflict_site, borrow_fact.borrow_kind
        );
    }
}

pub fn dump_binding_table(table: &BindingTable, file_set: &FileSet) {
    if !analysis_debug_enabled() {
        return;
    }

    eprintln!("BindingTable:");
    for record in table.iter_records() {
        let decl_site = render_span(file_set, record.decl_span);
        let state = match record.state {
            LivenessState::Live => "Live".to_owned(),
            LivenessState::Moved { at } => format!("Moved@{}", render_span(file_set, at)),
            LivenessState::Borrowed { at, kind } => {
                format!("Borrowed({kind:?})@{}", render_span(file_set, at))
            }
        };
        eprintln!("  {} {} {} {}", record.id.0 .0, record.name, decl_site, state);
    }
}

fn analysis_debug_enabled() -> bool {
    std::env::var("KOBO_ANALYSIS_DEBUG").as_deref() == Ok("1")
}

fn render_span(file_set: &FileSet, span: kobo_ir::KoboSpan) -> String {
    let Some(file) = file_set.get(span.file_id) else {
        return "<unknown>:1:1".to_owned();
    };
    let (line, column) = file.line_col(span.start);
    format!("{}:{line}:{column}", file.path.display())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::analysis_debug_enabled;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn analysis_debug_hook_is_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");

        std::env::remove_var("KOBO_ANALYSIS_DEBUG");
        assert!(!analysis_debug_enabled());

        std::env::set_var("KOBO_ANALYSIS_DEBUG", "1");
        assert!(analysis_debug_enabled());

        std::env::remove_var("KOBO_ANALYSIS_DEBUG");
        assert!(!analysis_debug_enabled());
    }
}
