use super::code_actions_for;
use kobo_errors::KErrorCode;

#[test]
fn active_k012x_codes_have_specific_lsp_actions() {
    for code in [
        KErrorCode::K0120,
        KErrorCode::K0121,
        KErrorCode::K0122,
        KErrorCode::K0123,
        KErrorCode::K0124,
        KErrorCode::K0125,
        KErrorCode::K0126,
        KErrorCode::K0127,
        KErrorCode::K0128,
        KErrorCode::K0129,
    ] {
        let actions = code_actions_for(code);
        assert!(
            actions.len() > 1,
            "{} should expose a code-specific action beyond Explain",
            code.as_str()
        );
        assert!(
            actions.iter().any(|action| action.group != "explain"),
            "{} should not fall back to explain-only LSP coverage",
            code.as_str()
        );
    }
}
