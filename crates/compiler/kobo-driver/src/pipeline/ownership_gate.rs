use kobo_analysis::{facts_to_diagnostics, run_analysis};
use kobo_errors::{KDiagnostic, KErrorCode, Severity};
use kobo_ir::{GuaranteePolicy, GuaranteeProfile, Kir, KoboSpan};

use crate::session::CompileSession;

pub(crate) fn project_blocking_ownership_diagnostics(session: &mut CompileSession, kir: &Kir) {
    let facts = run_analysis(kir, session.file_set());
    let release_policy = GuaranteePolicy::for_profile(GuaranteeProfile::Release);
    for mut diagnostic in facts_to_diagnostics(
        &facts,
        kir.transform_facts(),
        session.file_set(),
        kir,
        &release_policy,
    ) {
        if !is_ownership_gate_code(diagnostic.code) {
            continue;
        }
        if upgrade_existing_gate_diagnostic(session, &diagnostic) {
            continue;
        }
        diagnostic.severity = Severity::Error;
        session.push_diagnostic(diagnostic);
    }
}

pub(crate) fn has_blocking_ownership_diagnostic(session: &CompileSession) -> bool {
    session.visible_diagnostics().any(|diagnostic| {
        is_ownership_gate_code(diagnostic.code) && diagnostic.severity == Severity::Error
    })
}

pub(crate) fn has_known_borrow_conflict_diagnostic(session: &CompileSession) -> bool {
    session
        .visible_diagnostics()
        .any(|diagnostic| diagnostic.code == KErrorCode::K0002)
}

fn is_ownership_gate_code(code: KErrorCode) -> bool {
    matches!(
        code,
        KErrorCode::K0001 | KErrorCode::K0002 | KErrorCode::K0032
    )
}

fn same_gate_site(left: &KDiagnostic, right: &KDiagnostic) -> bool {
    left.code == right.code && same_span(left.primary.span, right.primary.span)
}

fn upgrade_existing_gate_diagnostic(
    session: &mut CompileSession,
    diagnostic: &KDiagnostic,
) -> bool {
    let Some(existing) = session
        .diagnostics
        .iter_mut()
        .find(|visible| same_gate_site(visible, diagnostic))
    else {
        return false;
    };

    existing.severity = Severity::Error;
    true
}

fn same_span(left: KoboSpan, right: KoboSpan) -> bool {
    left.file_id == right.file_id && left.start == right.start && left.end == right.end
}
