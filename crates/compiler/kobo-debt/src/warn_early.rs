use kobo_ir::debt::{DebtReport, WarnEarlyFact, WarnEarlyPattern};

/// Warn-early facts organised by pattern variant.
///
/// Active (non-suppressed) facts are split by variant type for rendering.
/// Suppressed facts are collected in `acknowledged`.
#[derive(Debug, Default)]
pub struct GroupedWarnings {
    /// K0080-P1 — bidirectional Rc links.
    pub bidirectional: Vec<WarnEarlyFact>,
    /// K0080-P2 — parent↔child back-pointer tree.
    pub parent_child: Vec<WarnEarlyFact>,
    /// K0080-P3 — shared mutable from 3+ call sites.
    pub shared_mutable: Vec<WarnEarlyFact>,
    /// K0080-P4 — self-referential struct without indirection.
    pub self_referential: Vec<WarnEarlyFact>,
    /// Facts suppressed by `#[kobo::known_debt = "..."]`.
    pub acknowledged: Vec<WarnEarlyFact>,
}

/// Group a flat list of `WarnEarlyFact` records by pattern variant.
///
/// Suppressed facts (`suppressed = true`) go into `acknowledged`; all others
/// are routed to the appropriate variant bucket.
pub fn group_by_pattern(facts: &[WarnEarlyFact]) -> GroupedWarnings {
    let mut grouped = GroupedWarnings::default();

    for fact in facts {
        if fact.suppressed {
            grouped.acknowledged.push(fact.clone());
            continue;
        }
        match &fact.pattern {
            WarnEarlyPattern::BidirectionalRcLinks { .. } => {
                grouped.bidirectional.push(fact.clone());
            }
            WarnEarlyPattern::ParentChildBackPointer { .. } => {
                grouped.parent_child.push(fact.clone());
            }
            WarnEarlyPattern::SharedMutableAt3PlusSites { .. } => {
                grouped.shared_mutable.push(fact.clone());
            }
            WarnEarlyPattern::SelfReferentialStruct { .. } => {
                grouped.self_referential.push(fact.clone());
            }
        }
    }

    grouped
}

/// Produce the "Structural warnings:" section of `kobo debt` output.
///
/// Returns an empty string when the report contains no active structural
/// warnings (the caller should omit the section header in that case).
pub fn format_warn_early(report: &DebtReport) -> String {
    let grouped = group_by_pattern(&report.warn_early);
    let total_active = grouped.bidirectional.len()
        + grouped.parent_child.len()
        + grouped.shared_mutable.len()
        + grouped.self_referential.len();

    if total_active == 0 && grouped.acknowledged.is_empty() {
        return String::new();
    }

    let mut out = String::new();

    if total_active > 0 {
        out.push_str("Structural warnings:\n");

        for fact in &grouped.bidirectional {
            if let WarnEarlyPattern::BidirectionalRcLinks {
                struct_name,
                field_pairs,
            } = &fact.pattern
            {
                out.push_str(&format!(
                    "  note[K0080-P1]: bidirectional Rc links in `{struct_name}`\n"
                ));
                for (a, b) in field_pairs {
                    out.push_str(&format!(
                        "  = fields `{a}` ↔ `{b}` form a bidirectional link\n"
                    ));
                }
                out.push_str(
                    "  = migration: consider Weak<T> for one direction or arena allocation\n",
                );
            }
        }

        for fact in &grouped.parent_child {
            if let WarnEarlyPattern::ParentChildBackPointer {
                struct_name,
                children_field,
                parent_field,
            } = &fact.pattern
            {
                out.push_str(&format!(
                    "  note[K0080-P2]: parent↔child back-pointer in `{struct_name}`\n"
                ));
                out.push_str(&format!(
                    "  = `{children_field}` (children) ↔ `{parent_field}` (parent)\n"
                ));
                out.push_str(
                    "  = migration: use Weak<T> for the parent direction to break the cycle\n",
                );
            }
        }

        for fact in &grouped.shared_mutable {
            if let WarnEarlyPattern::SharedMutableAt3PlusSites {
                binding_id,
                site_count,
            } = &fact.pattern
            {
                out.push_str(&format!(
                    "  note[K0080-P3]: shared mutable state at {site_count} call sites \
                     (binding {:?})\n",
                    binding_id
                ));
                out.push_str(
                    "  = migration: consider actor pattern or message-passing interface\n",
                );
            }
        }

        for fact in &grouped.self_referential {
            if let WarnEarlyPattern::SelfReferentialStruct { struct_name } = &fact.pattern {
                out.push_str(&format!(
                    "  note[K0080-P4]: self-referential struct `{struct_name}` \
                     without Rc/Box indirection\n"
                ));
                out.push_str(
                    "  = migration: wrap the recursive field in Box<T> or Rc<T>\n",
                );
            }
        }
    }

    if !grouped.acknowledged.is_empty() {
        out.push_str("Acknowledged debt:\n");
        for fact in &grouped.acknowledged {
            let reason = fact
                .known_debt_reason
                .as_deref()
                .unwrap_or("<no reason provided>");
            let code = pattern_code(&fact.pattern);
            out.push_str(&format!(
                "  [{code}] `{}` — {reason}\n",
                fact.struct_name
            ));
        }
    }

    out
}

fn pattern_code(pattern: &WarnEarlyPattern) -> &'static str {
    match pattern {
        WarnEarlyPattern::BidirectionalRcLinks { .. } => "K0080-P1",
        WarnEarlyPattern::ParentChildBackPointer { .. } => "K0080-P2",
        WarnEarlyPattern::SharedMutableAt3PlusSites { .. } => "K0080-P3",
        WarnEarlyPattern::SelfReferentialStruct { .. } => "K0080-P4",
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::debt::WarnEarlyPattern;
    use kobo_ir::{FileId, KirNodeId, KoboSpan};

    fn dummy_span() -> KoboSpan {
        KoboSpan::new(0, 0, FileId(0))
    }

    fn p1_fact(suppressed: bool) -> WarnEarlyFact {
        WarnEarlyFact {
            node_id: KirNodeId(1),
            span: dummy_span(),
            pattern: WarnEarlyPattern::BidirectionalRcLinks {
                struct_name: "Node".to_owned(),
                field_pairs: vec![("parent".to_owned(), "child".to_owned())],
            },
            suppressed,
            known_debt_reason: if suppressed {
                Some("will use arena".to_owned())
            } else {
                None
            },
            struct_name: "Node".to_owned(),
        }
    }

    fn p2_fact(suppressed: bool) -> WarnEarlyFact {
        WarnEarlyFact {
            node_id: KirNodeId(2),
            span: dummy_span(),
            pattern: WarnEarlyPattern::ParentChildBackPointer {
                struct_name: "Tree".to_owned(),
                children_field: "children".to_owned(),
                parent_field: "parent".to_owned(),
            },
            suppressed,
            known_debt_reason: if suppressed {
                Some("known tree pattern".to_owned())
            } else {
                None
            },
            struct_name: "Tree".to_owned(),
        }
    }

    #[test]
    fn empty_facts_produce_empty_string() {
        let report = DebtReport::new();
        let out = format_warn_early(&report);
        assert!(out.is_empty(), "no structural warnings section expected");
    }

    #[test]
    fn active_p1_appears_in_structural_warnings() {
        let mut report = DebtReport::new();
        report.warn_early.push(p1_fact(false));
        let out = format_warn_early(&report);
        assert!(out.contains("K0080-P1"), "should mention K0080-P1");
        assert!(out.contains("Node"), "should mention struct name");
        assert!(!out.contains("Acknowledged"), "no acknowledged section expected");
    }

    #[test]
    fn suppressed_p2_appears_in_acknowledged() {
        let mut report = DebtReport::new();
        report.warn_early.push(p1_fact(false));
        // Suppressed fact goes to acknowledged
        report.acknowledged.push(kobo_ir::debt::AcknowledgedDebtRecord {
            node_id: KirNodeId(0),
            span: dummy_span(),
            reason: "known tree pattern".to_owned(),
            pattern: WarnEarlyPattern::ParentChildBackPointer {
                struct_name: "Tree".to_owned(),
                children_field: "children".to_owned(),
                parent_field: "parent".to_owned(),
            },
            struct_name: "Tree".to_owned(),
        });
        let grouped = group_by_pattern(&[p1_fact(false), p2_fact(true)]);
        assert_eq!(grouped.bidirectional.len(), 1);
        assert_eq!(grouped.acknowledged.len(), 1);
    }

    #[test]
    fn group_by_pattern_routes_correctly() {
        let facts = [p1_fact(false), p2_fact(false), p1_fact(true)];
        let grouped = group_by_pattern(&facts);
        assert_eq!(grouped.bidirectional.len(), 1);
        assert_eq!(grouped.parent_child.len(), 1);
        assert_eq!(grouped.acknowledged.len(), 1);
        assert_eq!(grouped.shared_mutable.len(), 0);
        assert_eq!(grouped.self_referential.len(), 0);
    }

    #[test]
    fn multi_pair_p1_renders_all_pairs() {
        let mut report = DebtReport::new();
        report.warn_early.push(WarnEarlyFact {
            node_id: KirNodeId(1),
            span: dummy_span(),
            pattern: WarnEarlyPattern::BidirectionalRcLinks {
                struct_name: "GraphNode".to_owned(),
                field_pairs: vec![
                    ("parent".to_owned(), "left".to_owned()),
                    ("parent".to_owned(), "right".to_owned()),
                    ("left".to_owned(), "right".to_owned()),
                ],
            },
            suppressed: false,
            known_debt_reason: None,
            struct_name: "GraphNode".to_owned(),
        });
        let out = format_warn_early(&report);
        assert!(out.contains("parent` ↔ `left"), "first pair missing");
        assert!(out.contains("parent` ↔ `right"), "second pair missing");
        assert!(out.contains("left` ↔ `right"), "third pair missing");
    }
}
