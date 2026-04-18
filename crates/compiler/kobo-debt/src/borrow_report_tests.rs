use kobo_ir::{BindingUsage, BorrowKind, FileId, KirNodeId, KoboSpan, SharedBindingFacts, UseEvent};

use crate::borrow_report::{build_borrow_report, BorrowConflictKind, BorrowFixPattern, BorrowReport};

fn span(start: u32, end: u32) -> KoboSpan {
    KoboSpan::new(start, end, FileId(0))
}

fn default_shared_facts(node_id: KirNodeId) -> SharedBindingFacts {
    SharedBindingFacts {
        node_id,
        mutation_required: false,
        escape_floor: None,
        borrow_sites: vec![],
        read_sites: 0,
        mutable_sites: 0,
        has_escape: false,
        needs_sharing: false,
        needs_mutable_wrapper: false,
        needs_send: false,
        live_borrow_at_move: false,
        sequential_read_only: false,
        box_reason: None,
    }
}

#[test]
fn borrow_overlap_detected_for_immut_and_mut() {
    // Simulate: let r1 = &data; let r2 = &mut data; use(r1);
    // Immutable borrow at span(10,20), mutable borrow at span(15,25) → overlap
    let usage = BindingUsage {
        declaration: span(0, 5),
        uses: vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10, 20),
            },
            UseEvent::Borrowed {
                kind: BorrowKind::Mutable,
                span: span(15, 25),
            },
        ],
    };
    let facts = SharedBindingFacts {
        borrow_sites: vec![span(10, 20), span(15, 25)],
        ..default_shared_facts(KirNodeId(1))
    };

    let report = build_borrow_report("data", &usage, &facts);
    assert!(
        !report.overlapping_sites.is_empty(),
        "should detect overlapping borrows"
    );
    assert!(report.overlapping_sites[0].fix_pattern.is_some());
}

#[test]
fn no_overlap_for_sequential_immutable_borrows() {
    // Two immutable borrows → no conflict
    let usage = BindingUsage {
        declaration: span(0, 5),
        uses: vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10, 20),
            },
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(25, 35),
            },
        ],
    };
    let facts = SharedBindingFacts {
        borrow_sites: vec![span(10, 20), span(25, 35)],
        ..default_shared_facts(KirNodeId(2))
    };

    let report = build_borrow_report("x", &usage, &facts);
    assert!(
        report.overlapping_sites.is_empty(),
        "immutable-only borrows should not conflict"
    );
}

#[test]
fn overlap_with_live_borrow_at_move() {
    // Borrow alive when value is moved → overlap
    let usage = BindingUsage {
        declaration: span(0, 5),
        uses: vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10, 30),
            },
            UseEvent::Moved {
                span: span(20, 25),
                scope_depth: 0,
            },
        ],
    };
    let facts = SharedBindingFacts {
        borrow_sites: vec![span(10, 30)],
        live_borrow_at_move: true,
        ..default_shared_facts(KirNodeId(3))
    };

    let report = build_borrow_report("v", &usage, &facts);
    assert!(
        !report.overlapping_sites.is_empty(),
        "live borrow at move should be reported"
    );
    assert!(matches!(
        report.overlapping_sites[0].fix_pattern,
        Some(BorrowFixPattern::Clone)
    ));
}

// ── BUG-24: BorrowConflictKind ──

#[test]
fn borrow_conflict_kind_serializes_correctly() {
    let kind = BorrowConflictKind::ReadWriteOverlap;
    let json = serde_json::to_string(&kind).unwrap();
    assert_eq!(json, "\"ReadWriteOverlap\"");

    let kind2 = BorrowConflictKind::MultiMutableAlias;
    let json2 = serde_json::to_string(&kind2).unwrap();
    assert_eq!(json2, "\"MultiMutableAlias\"");

    let kind3 = BorrowConflictKind::LifetimeEscape;
    let json3 = serde_json::to_string(&kind3).unwrap();
    assert_eq!(json3, "\"LifetimeEscape\"");
}

#[test]
fn overlap_report_tags_read_write_conflict_kind() {
    let usage = BindingUsage {
        declaration: span(0, 5),
        uses: vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10, 30),
            },
            UseEvent::Borrowed {
                kind: BorrowKind::Mutable,
                span: span(15, 25),
            },
        ],
    };
    let facts = SharedBindingFacts {
        borrow_sites: vec![span(10, 30), span(15, 25)],
        ..default_shared_facts(KirNodeId(10))
    };

    let report = build_borrow_report("x", &usage, &facts);
    assert!(!report.overlapping_sites.is_empty());
    assert_eq!(
        report.overlapping_sites[0].conflict_kind,
        BorrowConflictKind::ReadWriteOverlap
    );
}

// ── BUG-9: BorrowReport schema_version + JSON round-trip ──

#[test]
fn borrow_report_has_schema_version() {
    let report = BorrowReport {
        schema_version: 1,
        total_bindings_analyzed: 0,
        bindings_with_overlaps: 0,
        overlapping_sites: vec![],
    };
    assert_eq!(report.schema_version, 1);
}

#[test]
fn borrow_report_round_trips_through_json() {
    let usage = BindingUsage {
        declaration: span(0, 5),
        uses: vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10, 30),
            },
            UseEvent::Borrowed {
                kind: BorrowKind::Mutable,
                span: span(15, 25),
            },
        ],
    };
    let facts = SharedBindingFacts {
        borrow_sites: vec![span(10, 30), span(15, 25)],
        ..default_shared_facts(KirNodeId(20))
    };

    let report = build_borrow_report("x", &usage, &facts);
    let json = serde_json::to_string_pretty(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert!(parsed.get("schema_version").is_some(), "JSON must contain schema_version");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed.get("overlapping_sites").is_some(), "JSON must contain overlapping_sites");
}
