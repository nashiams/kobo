use kobo_ir::{FileSet, Kir, NodeKind};

use crate::debug;
use crate::liveness::{binding_name, BindingId, BindingTable, LexicalEnv};
use crate::ownership_facts::{BorrowFact, MoveFact};
use crate::passes::{borrow_check, move_check};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnalysisFacts {
    pub moves: Vec<MoveFact>,
    pub borrows: Vec<BorrowFact>,
}

pub fn run_analysis(kir: &Kir, file_set: &FileSet) -> AnalysisFacts {
    let mut lexical_env = LexicalEnv::default();
    let mut binding_table = BindingTable::default();
    let mut facts = AnalysisFacts::default();

    // A single walk drives both passes. Borrow checking must observe the
    // moved/borrowed state mutated by move checking for the same source-order
    // node sequence.
    for node in kir.nodes_in_source_order() {
        match node.kind {
            NodeKind::ScopeStart => lexical_env.push_scope(),
            NodeKind::ScopeEnd => lexical_env.pop_scope(&mut binding_table),
            NodeKind::Decl => {
                let binding_id = BindingId(node.id);
                lexical_env.declare_binding(binding_id);
                binding_table.declare_binding(binding_id, binding_name(file_set, node.span), node.span);
            }
            _ => {}
        }

        move_check::visit(node, &mut binding_table, &mut facts.moves);
        borrow_check::visit(node, &mut binding_table, &mut facts.borrows);

        if !matches!(node.kind, NodeKind::Decl) {
            binding_table.clear_pending_decl();
        }
    }

    debug::dump_facts(&facts, file_set);
    debug::dump_binding_table(&binding_table, file_set);
    facts
}

#[cfg(test)]
mod tests {
    use kobo_ir::{
        BorrowKind, FileId, FileSet, FileSetBuilder, Kir, KirNode, KirNodeId, KoboSpan, NodeKind,
        OwnershipTier, UseKind,
    };

    use crate::{run_analysis, AnalysisFacts};

    fn build_file(source: &str) -> (FileSet, FileId) {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file("test.kobo".into(), source.to_owned());
        (file_set_builder.finish(), file_id)
    }

    fn node(id: u32, kind: NodeKind, span: KoboSpan, decl_id: Option<u32>) -> KirNode {
        KirNode {
            id: KirNodeId(id),
            kind,
            ast_id: None,
            ownership: OwnershipTier::RcMutShared,
            resource_kind: None,
            cfg_block: None,
            span,
            decl_id: decl_id.map(KirNodeId),
        }
    }

    fn decl(id: u32, span: KoboSpan) -> KirNode {
        KirNode {
            id: KirNodeId(id),
            kind: NodeKind::Decl,
            ast_id: None,
            ownership: OwnershipTier::RcMutShared,
            resource_kind: None,
            cfg_block: None,
            span,
            decl_id: None,
        }
    }

    fn scope_start(id: u32, span: KoboSpan) -> KirNode {
        node(id, NodeKind::ScopeStart, span, None)
    }

    fn scope_end(id: u32, span: KoboSpan) -> KirNode {
        node(id, NodeKind::ScopeEnd, span, None)
    }

    #[test]
    fn move_detection_finds_use_after_move() {
        let source = "let config = setup();\nprocess(config);\nlog(config);\n";
        let (file_set, file_id) = build_file(source);
        let kir = Kir::from_nodes(vec![
            scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
            decl(2, KoboSpan::new(4, 10, file_id)),
            node(3, NodeKind::Move, KoboSpan::new(29, 35, file_id), Some(2)),
            node(4, NodeKind::Use(UseKind::Read), KoboSpan::new(43, 49, file_id), Some(2)),
            scope_end(5, KoboSpan::new(0, source.len() as u32, file_id)),
        ]);

        let facts = run_analysis(&kir, &file_set);

        assert_eq!(facts.moves.len(), 1);
        assert_eq!(facts.moves[0].move_site, KoboSpan::new(29, 35, file_id));
        assert_eq!(facts.moves[0].later_use, KoboSpan::new(43, 49, file_id));
        assert!(facts.borrows.is_empty());
    }

    #[test]
    fn move_detection_skips_two_line_non_conflict() {
        let source = "let config = setup();\nprocess(config);\n";
        let (file_set, file_id) = build_file(source);
        let kir = Kir::from_nodes(vec![
            scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
            decl(2, KoboSpan::new(4, 10, file_id)),
            node(3, NodeKind::Move, KoboSpan::new(29, 35, file_id), Some(2)),
            scope_end(4, KoboSpan::new(0, source.len() as u32, file_id)),
        ]);

        let facts = run_analysis(&kir, &file_set);
        assert_eq!(facts, AnalysisFacts::default());
    }

    #[test]
    fn borrow_detection_finds_mutable_use_after_immutable_borrow() {
        let source = "let data = vec![1, 2, 3];\nlet r = &data;\ndata.push(4);\n";
        let (file_set, file_id) = build_file(source);
        let kir = Kir::from_nodes(vec![
            scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
            decl(2, KoboSpan::new(4, 8, file_id)),
            decl(3, KoboSpan::new(31, 32, file_id)),
            node(
                4,
                NodeKind::Borrow(BorrowKind::Immutable),
                KoboSpan::new(36, 40, file_id),
                Some(2),
            ),
            node(5, NodeKind::Use(UseKind::Write), KoboSpan::new(42, 46, file_id), Some(2)),
            scope_end(6, KoboSpan::new(0, source.len() as u32, file_id)),
        ]);

        let facts = run_analysis(&kir, &file_set);

        assert!(facts.moves.is_empty());
        assert_eq!(facts.borrows.len(), 1);
        assert_eq!(facts.borrows[0].borrow_site, KoboSpan::new(36, 40, file_id));
        assert_eq!(facts.borrows[0].conflict_site, KoboSpan::new(42, 46, file_id));
    }

    #[test]
    fn borrow_detection_drops_alias_when_scope_ends() {
        let source = "let data = vec![1, 2, 3];\n{\n    let r = &data;\n}\ndata.push(4);\n";
        let (file_set, file_id) = build_file(source);
        let kir = Kir::from_nodes(vec![
            scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
            decl(2, KoboSpan::new(4, 8, file_id)),
            scope_start(3, KoboSpan::new(27, 51, file_id)),
            decl(4, KoboSpan::new(37, 38, file_id)),
            node(
                5,
                NodeKind::Borrow(BorrowKind::Immutable),
                KoboSpan::new(42, 46, file_id),
                Some(2),
            ),
            scope_end(6, KoboSpan::new(27, 51, file_id)),
            node(7, NodeKind::Use(UseKind::Write), KoboSpan::new(52, 56, file_id), Some(2)),
            scope_end(8, KoboSpan::new(0, source.len() as u32, file_id)),
        ]);

        let facts = run_analysis(&kir, &file_set);
        assert!(facts.moves.is_empty());
        assert!(facts.borrows.is_empty());
    }

    #[test]
    fn move_while_borrowed_emits_only_move_fact() {
        let source = "let data = vec![1, 2, 3];\nlet r = &data;\nconsume(data);\n";
        let (file_set, file_id) = build_file(source);
        let kir = Kir::from_nodes(vec![
            scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
            decl(2, KoboSpan::new(4, 8, file_id)),
            decl(3, KoboSpan::new(31, 32, file_id)),
            node(
                4,
                NodeKind::Borrow(BorrowKind::Immutable),
                KoboSpan::new(36, 40, file_id),
                Some(2),
            ),
            node(5, NodeKind::Move, KoboSpan::new(50, 54, file_id), Some(2)),
            scope_end(6, KoboSpan::new(0, source.len() as u32, file_id)),
        ]);

        let facts = run_analysis(&kir, &file_set);
        assert_eq!(facts.moves.len(), 1);
        assert!(facts.borrows.is_empty());
    }

    #[test]
    fn inner_shadowing_does_not_affect_outer_binding() {
        let source = "let x = make();\n{\n    let x = make();\n    consume(x);\n}\nuse_outer(x);\n";
        let (file_set, file_id) = build_file(source);
        let kir = Kir::from_nodes(vec![
            scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
            decl(2, KoboSpan::new(4, 5, file_id)),
            scope_start(3, KoboSpan::new(16, 58, file_id)),
            decl(4, KoboSpan::new(26, 27, file_id)),
            node(5, NodeKind::Move, KoboSpan::new(48, 49, file_id), Some(4)),
            scope_end(6, KoboSpan::new(16, 58, file_id)),
            node(7, NodeKind::Use(UseKind::Read), KoboSpan::new(69, 70, file_id), Some(2)),
            scope_end(8, KoboSpan::new(0, source.len() as u32, file_id)),
        ]);

        let facts = run_analysis(&kir, &file_set);
        assert!(facts.moves.is_empty());
        assert!(facts.borrows.is_empty());
    }
}
