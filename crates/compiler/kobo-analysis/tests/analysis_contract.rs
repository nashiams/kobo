use kobo_analysis::{run_analysis, BorrowFact, MoveFact};
use kobo_ir::{
    BorrowKind, FileId, FileSet, FileSetBuilder, Kir, KirNode, KirNodeId, KoboSpan, NodeKind,
    OwnershipTier, UseKind,
};

fn build_file(source: &str) -> (FileSet, FileId) {
    let mut file_set_builder = FileSetBuilder::new();
    let file_id = file_set_builder.add_file("analysis_contract.kobo".into(), source.to_owned());
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
    node(id, NodeKind::Decl, span, None)
}

fn scope_start(id: u32, span: KoboSpan) -> KirNode {
    node(id, NodeKind::ScopeStart, span, None)
}

fn scope_end(id: u32, span: KoboSpan) -> KirNode {
    node(id, NodeKind::ScopeEnd, span, None)
}

#[test]
fn k0001_exact_fixture_produces_move_fact() {
    let source =
        "fn main() {\n    let config = make_config();\n    process(config);\n    log(config);\n}\n";
    let (file_set, file_id) = build_file(source);
    let kir = Kir::from_nodes(vec![
        scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
        decl(2, KoboSpan::new(20, 26, file_id)),
        node(3, NodeKind::Move, KoboSpan::new(58, 64, file_id), Some(2)),
        node(
            4,
            NodeKind::Use(UseKind::Read),
            KoboSpan::new(77, 83, file_id),
            Some(2),
        ),
        scope_end(5, KoboSpan::new(0, source.len() as u32, file_id)),
    ]);

    let facts = run_analysis(&kir, &file_set);
    assert_eq!(
        facts.moves,
        vec![MoveFact {
            binding: KirNodeId(2),
            move_site: KoboSpan::new(58, 64, file_id),
            later_use: KoboSpan::new(77, 83, file_id),
        }]
    );
    assert!(facts.borrows.is_empty());
}

#[test]
fn k0002_exact_fixture_produces_borrow_fact() {
    let source =
        "fn main() {\n    let data = vec![1, 2, 3];\n    let r = &data;\n    data.push(4);\n}\n";
    let (file_set, file_id) = build_file(source);
    let kir = Kir::from_nodes(vec![
        scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
        decl(2, KoboSpan::new(20, 24, file_id)),
        decl(3, KoboSpan::new(52, 53, file_id)),
        node(
            4,
            NodeKind::Borrow(BorrowKind::Immutable),
            KoboSpan::new(57, 61, file_id),
            Some(2),
        ),
        node(
            5,
            NodeKind::Use(UseKind::Write),
            KoboSpan::new(67, 71, file_id),
            Some(2),
        ),
        scope_end(6, KoboSpan::new(0, source.len() as u32, file_id)),
    ]);

    let facts = run_analysis(&kir, &file_set);
    assert!(facts.moves.is_empty());
    assert_eq!(
        facts.borrows,
        vec![BorrowFact {
            binding: KirNodeId(2),
            borrow_site: KoboSpan::new(57, 61, file_id),
            conflict_site: KoboSpan::new(67, 71, file_id),
            borrow_kind: BorrowKind::Immutable,
        }]
    );
}

#[test]
fn moved_while_borrowed_drops_spurious_borrow_conflict() {
    let source =
        "fn main() {\n    let data = vec![1, 2, 3];\n    let r = &data;\n    consume(data);\n}\n";
    let (file_set, file_id) = build_file(source);
    let kir = Kir::from_nodes(vec![
        scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
        decl(2, KoboSpan::new(20, 24, file_id)),
        decl(3, KoboSpan::new(52, 53, file_id)),
        node(
            4,
            NodeKind::Borrow(BorrowKind::Immutable),
            KoboSpan::new(57, 61, file_id),
            Some(2),
        ),
        node(5, NodeKind::Move, KoboSpan::new(74, 78, file_id), Some(2)),
        scope_end(6, KoboSpan::new(0, source.len() as u32, file_id)),
    ]);

    let facts = run_analysis(&kir, &file_set);
    assert_eq!(facts.moves.len(), 1);
    assert!(facts.borrows.is_empty());
}

#[test]
fn nested_shadowing_keeps_distinct_binding_ids() {
    let source = "fn main() {\n    let x = make();\n    {\n        let x = make();\n        consume(x);\n    }\n    use_outer(x);\n}\n";
    let (file_set, file_id) = build_file(source);
    let kir = Kir::from_nodes(vec![
        scope_start(1, KoboSpan::new(0, source.len() as u32, file_id)),
        decl(2, KoboSpan::new(20, 21, file_id)),
        scope_start(3, KoboSpan::new(38, 91, file_id)),
        decl(4, KoboSpan::new(52, 53, file_id)),
        node(5, NodeKind::Move, KoboSpan::new(76, 77, file_id), Some(4)),
        scope_end(6, KoboSpan::new(38, 91, file_id)),
        node(
            7,
            NodeKind::Use(UseKind::Read),
            KoboSpan::new(106, 107, file_id),
            Some(2),
        ),
        scope_end(8, KoboSpan::new(0, source.len() as u32, file_id)),
    ]);

    let facts = run_analysis(&kir, &file_set);
    assert!(facts.moves.is_empty());
    assert!(facts.borrows.is_empty());
}
