use kobo_errors::KErrorCode;

pub(super) fn label_for_code(code: KErrorCode) -> &'static str {
    match code {
        KErrorCode::K0100 => "this obligation can leave without finishing",
        KErrorCode::K0102 => "this replay path can change between runs",
        KErrorCode::K0105 => "simulation event budget exceeded",
        KErrorCode::K0107 => "external replay boundary requires policy",
        KErrorCode::K0116 => "scenario coverage is incomplete",
        KErrorCode::K0117 => "semantic and harness traces diverged",
        KErrorCode::K0118 => "evidence artifact is stale",
        KErrorCode::K0120 => "ecosystem policy metadata is invalid",
        KErrorCode::K0121 => "declaration metadata is invalid or stale",
        KErrorCode::K0122 => "typed external boundary has no declaration",
        KErrorCode::K0123 => "adapter package is missing or incompatible",
        KErrorCode::K0124 => "record boundary lacks recorded evidence",
        KErrorCode::K0125 => "activity declaration lacks retry metadata",
        KErrorCode::K0126 => ".kobo-summary hash or version mismatched",
        KErrorCode::K0127 => "bindgen declaration draft needs review",
        KErrorCode::K0128 => "Cargo compatibility metadata changed",
        KErrorCode::K0129 => "ecosystem replay evidence would overclaim coverage",
        _ => "simulation invariant failed",
    }
}

pub(super) fn decision_for_code(code: KErrorCode) -> &'static str {
    match code {
        KErrorCode::K0100 => {
            "call the required action on every path or pass the obligation on as debt"
        }
        KErrorCode::K0102 => {
            "route time or randomness through a modeled source before claiming replay"
        }
        KErrorCode::K0105 => "raise the event budget or remove the unbounded scenario loop",
        KErrorCode::K0107 => "select a boundary policy before exact replay",
        KErrorCode::K0116 => "keep the witness partial until the scenario coverage is modeled",
        KErrorCode::K0117 => "regenerate the witness and investigate the trace mismatch",
        KErrorCode::K0118 => "regenerate artifacts for the current source hash",
        KErrorCode::K0120 => "fix the [ecosystem] table before applying boundary policies",
        KErrorCode::K0121 => "fix or regenerate the declaration file",
        KErrorCode::K0122 => "add a declaration file or choose a non-typed boundary policy",
        KErrorCode::K0123 => "install the adapter package or choose record/activity/opaque/debt",
        KErrorCode::K0124 => "regenerate a recorded witness or choose another boundary policy",
        KErrorCode::K0125 => "record retry, idempotency, and compensation metadata",
        KErrorCode::K0126 => "rebuild the upstream summary and update the pinned hash",
        KErrorCode::K0127 => "review and complete generated declaration metadata",
        KErrorCode::K0128 => "preserve Cargo metadata or stop with a compatibility error",
        KErrorCode::K0129 => "keep witness partial or add matching boundary evidence",
        _ => "inspect the witness trace and update the scenario",
    }
}
