use kobo_ir::debt::{DebtReport, WarnEarlyPattern};

pub(super) fn migration_estimate_json(report: &DebtReport) -> serde_json::Value {
    serde_json::json!({
        "label": migration_estimate_label(report),
        "estimated_site_count": report.inventory.rc_mut_shared,
        "tier1": report.complexity.tier1,
        "tier2": report.complexity.tier2,
        "tier3": report.complexity.tier3,
        "precursor_warning_count": report.warn_early.len(),
    })
}

pub(super) fn migration_estimate_label(report: &DebtReport) -> &'static str {
    if report.complexity.tier3 > 0 {
        "estimated-high"
    } else if report.complexity.tier2 > 0 || !report.warn_early.is_empty() {
        "estimated-medium"
    } else {
        "estimated-low"
    }
}

pub(super) fn precursor_warning_json(report: &DebtReport) -> Vec<serde_json::Value> {
    report
        .warn_early
        .iter()
        .map(|fact| {
            serde_json::json!({
                "code": warn_early_code(&fact.pattern),
                "pattern": fact.pattern.clone(),
                "struct_name": fact.struct_name.clone(),
                "suppressed": fact.suppressed,
            })
        })
        .collect()
}

pub(super) fn precursor_codes(report: &DebtReport) -> Vec<&'static str> {
    let mut codes = report
        .warn_early
        .iter()
        .map(|fact| warn_early_code(&fact.pattern))
        .collect::<Vec<_>>();
    codes.sort_unstable();
    codes.dedup();
    codes
}

fn warn_early_code(pattern: &WarnEarlyPattern) -> &'static str {
    match pattern {
        WarnEarlyPattern::BidirectionalRcLinks { .. } => "K0080-P1",
        WarnEarlyPattern::ParentChildBackPointer { .. } => "K0080-P2",
        WarnEarlyPattern::SharedMutableAt3PlusSites { .. } => "K0080-P3",
        WarnEarlyPattern::SelfReferentialStruct { .. } => "K0080-P4",
    }
}
