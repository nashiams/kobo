use std::fmt;

use kobo_ir::KoboMode;

macro_rules! define_error_codes {
    ($( $code:ident => $label:literal, )* ) => {
        #[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
        pub enum KErrorCode {
            $( $code, )*
        }

        impl KErrorCode {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$code => $label, )*
                }
            }

            pub const fn short_description(self) -> &'static str {
                match self {
                    Self::K0001 => "value used after move",
                    Self::K0002 => "cannot borrow as mutable - already borrowed",
                    Self::K0020 => "RefCell accessed >10,000 times in hot path",
                    Self::K0021 => "DiagOwner borrow counter saturated — count understated",
                    Self::K0025 => "soft hint ignored - constraint conflict",
                    Self::K0030 => "resource handle moved - cannot alias file handle",
                    Self::K0041 => "cannot enter @strict block - value has active aliases",
                    Self::K0042 => "closure captures LocalOwned<T> across @strict boundary",
                    Self::K0043 => "value moved inside @strict block - cannot re-wrap on exit",
                    Self::K0060 => "RefCell borrow is live at suspend point",
                    Self::K0061 => "future requires Send but value cannot safely cross thread boundary",
                    Self::K0062 => "Mutex guard would live across .await",
                    Self::K0063 => "@strict block inside async fn without @strict async fn",
                    Self::K0064 => "@strict inside async block — ownership cannot be tracked across yield",
                    Self::K0065 => "@strict with generator/coroutine — ownership cannot cross yield point",
                    Self::K0067 => "spawn_local requires LocalSet executor context",
                    Self::K0080 => "structural ownership conflict - no automatic fix possible",
                    Self::K0080P1 => "ownership pattern will require architectural decision at migration",
                    Self::K0080P2 => "parent↔child Rc back-pointer tree — cycle risk",
                    Self::K0080P3 => "shared mutable state at 3+ call sites",
                    Self::K0080P4 => "self-referential struct — infinite size without indirection",
                    Self::K0081 => "ownership cluster too large for automatic solving",
                    Self::K0082 => "solver exceeded its time budget",
                    Self::K0083 => "solver decision requires human review",
                    Self::K0084 => "solver made a provisional decision with medium confidence",
                    Self::K0085 => "solver applied a low-confidence heuristic — verify manually",
                    Self::K0090 => "migration cannot continue - value crosses into external crate",
                    Self::K0095 => "ownership of macro-generated value cannot be inferred",
                    Self::K0099 => "rustc error remapped to Kobo source",
                    _ => "diagnostic stub",
                }
            }
        }

        impl fmt::Display for KErrorCode {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct KErrorMetadata {
    pub short_description: &'static str,
}

define_error_codes! {
    K0001 => "K0001",
    K0002 => "K0002",
    K0003 => "K0003",
    K0004 => "K0004",
    K0005 => "K0005",
    K0006 => "K0006",
    K0007 => "K0007",
    K0008 => "K0008",
    K0009 => "K0009",
    K0010 => "K0010",
    K0011 => "K0011",
    K0012 => "K0012",
    K0013 => "K0013",
    K0014 => "K0014",
    K0015 => "K0015",
    K0016 => "K0016",
    K0017 => "K0017",
    K0018 => "K0018",
    K0019 => "K0019",
    K0020 => "K0020",
    K0021 => "K0021",
    K0022 => "K0022",
    K0023 => "K0023",
    K0024 => "K0024",
    K0025 => "K0025",
    K0026 => "K0026",
    K0027 => "K0027",
    K0028 => "K0028",
    K0029 => "K0029",
    K0030 => "K0030",
    K0031 => "K0031",
    K0032 => "K0032",
    K0033 => "K0033",
    K0034 => "K0034",
    K0035 => "K0035",
    K0036 => "K0036",
    K0037 => "K0037",
    K0038 => "K0038",
    K0039 => "K0039",
    K0040 => "K0040",
    K0041 => "K0041",
    K0042 => "K0042",
    K0043 => "K0043",
    // TODO(v0.7): K0044 — labeled break/continue across @strict boundary
    K0044 => "K0044",
    // TODO(v0.7): K0045 — @strict across module boundary
    K0045 => "K0045",
    // TODO(v0.7): K0046 — @strict with unsafe block interaction
    K0046 => "K0046",
    K0047 => "K0047",
    K0048 => "K0048",
    K0049 => "K0049",
    K0050 => "K0050",
    K0051 => "K0051",
    K0052 => "K0052",
    K0053 => "K0053",
    K0054 => "K0054",
    K0055 => "K0055",
    K0056 => "K0056",
    K0057 => "K0057",
    K0058 => "K0058",
    K0059 => "K0059",
    K0060 => "K0060",
    K0061 => "K0061",
    K0062 => "K0062",
    K0063 => "K0063",
    // TODO(v0.7): K0064 — @strict inside async block (currently folded into K0063)
    K0064 => "K0064",
    // TODO(v0.8+): K0065 — @strict with generator/coroutine
    K0065 => "K0065",
    K0066 => "K0066",
    K0067 => "K0067",
    K0068 => "K0068",
    K0069 => "K0069",
    K0070 => "K0070",
    K0071 => "K0071",
    K0072 => "K0072",
    K0073 => "K0073",
    K0074 => "K0074",
    K0075 => "K0075",
    K0076 => "K0076",
    K0077 => "K0077",
    K0078 => "K0078",
    K0079 => "K0079",
    K0080 => "K0080",
    K0080P1 => "K0080-P1",
    K0080P2 => "K0080-P2",
    K0080P3 => "K0080-P3",
    K0080P4 => "K0080-P4",
    K0081 => "K0081",
    K0082 => "K0082",
    K0083 => "K0083",
    K0084 => "K0084",
    K0085 => "K0085",
    K0086 => "K0086",
    K0087 => "K0087",
    K0088 => "K0088",
    K0089 => "K0089",
    K0090 => "K0090",
    K0091 => "K0091",
    K0092 => "K0092",
    K0093 => "K0093",
    K0094 => "K0094",
    K0095 => "K0095",
    K0096 => "K0096",
    K0097 => "K0097",
    K0098 => "K0098",
    K0099 => "K0099",
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl KErrorCode {
    pub const fn metadata(self) -> KErrorMetadata {
        match self {
            Self::K0001 => KErrorMetadata {
                short_description: "value used after move",
            },
            Self::K0002 => KErrorMetadata {
                short_description: "cannot borrow as mutable - already borrowed",
            },
            _ => KErrorMetadata {
                short_description: self.short_description(),
            },
        }
    }
}

/// Map a compile mode to the severity used for ownership-class K-codes.
/// Script → None (silent), Checked → Warning, Strict → Error.
/// Not a method on KoboMode because KoboMode lives in kobo-ir and must not
/// depend on Severity (kobo-errors). No orphan impl allowed.
fn ownership_severity(mode: KoboMode) -> Option<Severity> {
    match mode {
        KoboMode::Script  => None,
        KoboMode::Checked => Some(Severity::Warning),
        KoboMode::Strict  => Some(Severity::Error),
    }
}

/// Async-specific severity: Script/Checked → Warning, Strict → Error.
fn async_severity(mode: KoboMode) -> Option<Severity> {
    match mode {
        KoboMode::Script  => Some(Severity::Warning),
        KoboMode::Checked => Some(Severity::Warning),
        KoboMode::Strict  => Some(Severity::Error),
    }
}

/// Resolve the severity of a K-code diagnostic based on compile mode.
///
/// Single source of truth for severity routing — Contract R08.
/// Returns `None` for diagnostics that should not be emitted in the given mode [R6-11].
/// None = do not create or emit the diagnostic at all.
pub fn resolve_severity(code: KErrorCode, mode: KoboMode) -> Option<Severity> {
    use KErrorCode::*;

    match code {
        // Ownership basics — mode-dependent.
        // Script: None (silent — no emission), Checked: Warning, Strict: Error [R6-11].
        K0001 | K0002 => ownership_severity(mode),

        // Performance advisories — always Warning in all modes.
        K0020 | K0021 => Some(Severity::Warning),

        // Constraint conflict — always Error (not a perf advisory).
        K0025 => Some(Severity::Error),

        // Resource handle — always Error.
        K0030 => Some(Severity::Error),

        // @strict boundary violations — always Error.
        K0041 | K0042 | K0043 => Some(Severity::Error),

        // Async ownership — mode-dependent.
        // Script/Checked: Warning, Strict: Error.
        K0060 | K0061 | K0062 | K0063 => async_severity(mode),

        // Structural advisory — always Note.
        K0080 => Some(Severity::Note),

        // Structural advisory precursors — always Note [R6-12].
        // These are structural advisories from the debt model, NOT mode-dependent
        // ownership enforcement. Do not route through ownership_severity().
        K0080P1 | K0080P2 | K0080P3 | K0080P4 => Some(Severity::Note),

        // Solver limits — always Error.
        K0081 | K0082 => Some(Severity::Error),

        // Cross-crate migration limits — always Error.
        K0090 => Some(Severity::Error),

        // Macro-generated inference failure — always Error.
        K0095 => Some(Severity::Error),

        // Rustc remap — always Error.
        K0099 => Some(Severity::Error),

        // Relax attribute advisory — always Warning [BUG-06].
        // Structural errors (malformed, non-function) use hardcoded Error
        // at the emission site (AC-19 exception for always-error validation).
        K0026 => Some(Severity::Warning),

        // Uncategorized ownership (catch-all for future rustc remapped codes).
        K0019 => ownership_severity(mode),

        // Stub codes — use mode-dependent ownership default.
        // When a stub becomes active, move it to its own explicit arm above.
        K0003 | K0004 | K0005 | K0006 | K0007 | K0008 | K0009 |
        K0010 | K0011 | K0012 | K0013 | K0014 | K0015 | K0016 |
        K0017 | K0018 |
        K0022 | K0023 | K0024 | K0027 | K0028 | K0029 |
        K0031 | K0032 | K0033 | K0034 | K0035 | K0036 | K0037 |
        K0038 | K0039 | K0040 | K0044 | K0045 | K0046 | K0047 |
        K0048 | K0049 | K0050 | K0051 | K0052 | K0053 | K0054 |
        K0055 | K0056 | K0057 | K0058 | K0059 |
        K0064 | K0065 | K0066 | K0067 | K0068 | K0069 |
        K0070 | K0071 | K0072 | K0073 | K0074 | K0075 | K0076 |
        K0077 | K0078 | K0079 | K0083 | K0084 |
        K0085 | K0086 | K0087 | K0088 | K0089 | K0091 | K0092 |
        K0093 | K0094 | K0096 | K0097 | K0098
            => ownership_severity(mode),
        // NO wildcard `_` arm — new variants cause compile error [R2-02].
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_severity, KErrorCode, Severity};
    use kobo_ir::KoboMode;

    #[test]
    fn precursor_code_uses_dash_suffix() {
        assert_eq!(KErrorCode::K0080P1.as_str(), "K0080-P1");
    }

    #[test]
    fn short_descriptions_match_v02_contract_for_move_and_borrow() {
        assert_eq!(
            KErrorCode::K0001.short_description(),
            "value used after move"
        );
        assert_eq!(
            KErrorCode::K0002.short_description(),
            "cannot borrow as mutable - already borrowed"
        );
        assert_eq!(
            KErrorCode::K0001.metadata().short_description,
            "value used after move"
        );
    }

    #[test]
    fn severity_formats_as_lowercase_label() {
        assert_eq!(Severity::Warning.as_str(), "warning");
    }

    // --- resolve_severity tests [G1 / Contract R08] ---

    #[test]
    fn k0001_script_is_silent() {
        assert_eq!(
            resolve_severity(KErrorCode::K0001, KoboMode::Script),
            None,
            "K0001 must not be emitted in script mode (R6-11)"
        );
    }

    #[test]
    fn k0001_checked_is_warning() {
        assert_eq!(
            resolve_severity(KErrorCode::K0001, KoboMode::Checked),
            Some(Severity::Warning)
        );
    }

    #[test]
    fn k0001_strict_is_error() {
        assert_eq!(
            resolve_severity(KErrorCode::K0001, KoboMode::Strict),
            Some(Severity::Error)
        );
    }

    #[test]
    fn k0002_script_is_silent() {
        assert_eq!(resolve_severity(KErrorCode::K0002, KoboMode::Script), None);
    }

    #[test]
    fn k0002_checked_is_warning() {
        assert_eq!(
            resolve_severity(KErrorCode::K0002, KoboMode::Checked),
            Some(Severity::Warning)
        );
    }

    #[test]
    fn perf_advisory_k0020_always_warning() {
        for mode in [KoboMode::Script, KoboMode::Checked, KoboMode::Strict] {
            assert_eq!(
                resolve_severity(KErrorCode::K0020, mode),
                Some(Severity::Warning),
                "K0020 must be Warning in all modes"
            );
        }
    }

    #[test]
    fn constraint_conflict_k0025_always_error() {
        for mode in [KoboMode::Script, KoboMode::Checked, KoboMode::Strict] {
            assert_eq!(
                resolve_severity(KErrorCode::K0025, mode),
                Some(Severity::Error)
            );
        }
    }

    #[test]
    fn strict_boundary_k0041_always_error() {
        for mode in [KoboMode::Script, KoboMode::Checked, KoboMode::Strict] {
            assert_eq!(
                resolve_severity(KErrorCode::K0041, mode),
                Some(Severity::Error)
            );
        }
    }

    #[test]
    fn async_k006x_script_and_checked_are_warnings_strict_is_error() {
        for code in [
            KErrorCode::K0060,
            KErrorCode::K0061,
            KErrorCode::K0062,
            KErrorCode::K0063,
        ] {
            assert_eq!(
                resolve_severity(code, KoboMode::Script),
                Some(Severity::Warning),
                "{code:?} must be a warning in script mode"
            );
            assert_eq!(
                resolve_severity(code, KoboMode::Checked),
                Some(Severity::Warning),
                "{code:?} must be a warning in checked mode"
            );
            assert_eq!(
                resolve_severity(code, KoboMode::Strict),
                Some(Severity::Error),
                "{code:?} must be an error in strict mode"
            );
        }
    }

    #[test]
    fn structural_advisory_k0080_always_note() {
        for mode in [KoboMode::Script, KoboMode::Checked, KoboMode::Strict] {
            assert_eq!(
                resolve_severity(KErrorCode::K0080, mode),
                Some(Severity::Note)
            );
        }
    }

    #[test]
    fn structural_advisory_precursors_always_note() {
        for code in [
            KErrorCode::K0080P1,
            KErrorCode::K0080P2,
            KErrorCode::K0080P3,
            KErrorCode::K0080P4,
        ] {
            for mode in [KoboMode::Script, KoboMode::Checked, KoboMode::Strict] {
                assert_eq!(
                    resolve_severity(code, mode),
                    Some(Severity::Note),
                    "{:?} must be Note in all modes (not mode-dependent) [R6-12]",
                    code
                );
            }
        }
    }

    #[test]
    fn rustc_remap_k0099_always_error() {
        for mode in [KoboMode::Script, KoboMode::Checked, KoboMode::Strict] {
            assert_eq!(
                resolve_severity(KErrorCode::K0099, mode),
                Some(Severity::Error)
            );
        }
    }

    #[test]
    fn k0026_is_always_warning() {
        assert_eq!(resolve_severity(KErrorCode::K0026, KoboMode::Script), Some(Severity::Warning));
        assert_eq!(resolve_severity(KErrorCode::K0026, KoboMode::Checked), Some(Severity::Warning));
        assert_eq!(resolve_severity(KErrorCode::K0026, KoboMode::Strict), Some(Severity::Warning));
    }
}
