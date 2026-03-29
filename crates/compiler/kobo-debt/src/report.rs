use kobo_ir::Kir;

// --- Types first ---

/// Read-only ownership cost report for a KIR instance.
///
/// Stub at v0.1: all fields are counters only. Full report format (§20) in v0.4.
#[derive(Debug, Default)]
pub struct DebtReport {
    pub total_nodes: usize,
    pub rc_mut_shared_count: usize,
    pub plain_owned_count: usize,
    pub undecided_count: usize,
}

// --- Functions ---

/// Produces a read-only ownership cost report from the frozen KIR.
///
/// Does not modify KIR. Called after `build_kir` and before `lower`.
pub fn debt_report(kir: &Kir) -> DebtReport {
    let mut report = DebtReport::default();
    report.total_nodes = kir.len();

    for node in kir.iter_nodes() {
        match node.ownership {
            kobo_ir::OwnershipTier::RcMutShared => report.rc_mut_shared_count += 1,
            kobo_ir::OwnershipTier::PlainOwned => report.plain_owned_count += 1,
            kobo_ir::OwnershipTier::Undecided => report.undecided_count += 1,
            _ => {}
        }
    }

    report
}
