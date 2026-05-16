use std::fmt;

use kobo_ir::GuaranteePolicy;

macro_rules! define_error_codes {
    ($( $code:ident => $label:literal, )* ) => {
        #[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
        pub enum KErrorCode {
            $( $code, )*
        }

        impl KErrorCode {
            pub const ALL: &'static [Self] = &[
                $( Self::$code, )*
            ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$code => $label, )*
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
    K0044 => "K0044",
    K0045 => "K0045",
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
    K0064 => "K0064",
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
    K0100 => "K0100",
    K0101 => "K0101",
    K0102 => "K0102",
    K0103 => "K0103",
    K0104 => "K0104",
    K0105 => "K0105",
    K0106 => "K0106",
    K0107 => "K0107",
    K0108 => "K0108",
    K0109 => "K0109",
    K0110 => "K0110",
    K0111 => "K0111",
    K0112 => "K0112",
    K0113 => "K0113",
    K0114 => "K0114",
    K0115 => "K0115",
    K0116 => "K0116",
    K0117 => "K0117",
    K0118 => "K0118",
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
    pub const fn short_description(self) -> &'static str {
        match self {
            Self::K0001 => "value used after move",
            Self::K0002 => "cannot borrow as mutable - already borrowed",
            Self::K0020 => "RefCell accessed >10,000 times in hot path",
            Self::K0021 => "DiagOwner borrow counter saturated - count understated",
            Self::K0025 => "Kobo cannot use this ownership hint",
            Self::K0026 => "relax attribute has no effect or is malformed",
            Self::K0030 => "resource handle moved - cannot alias file handle",
            Self::K0031 => "engine-owned binding capped to PlainOwned",
            Self::K0032 => "live borrow at move forces shared ownership",
            Self::K0041 => "cannot enter @strict block - value has active aliases",
            Self::K0042 => "closure captures value across @strict boundary",
            Self::K0043 => "value moved inside @strict block - cannot restore on exit",
            Self::K0044 => "labeled break or continue crosses @strict boundary",
            Self::K0060 => "RefCell borrow is live at suspend point",
            Self::K0061 => "future requires Send but value cannot safely cross thread boundary",
            Self::K0062 => "async executor dependency is missing",
            Self::K0063 => "this strict borrow is inside code that can pause",
            Self::K0064 => "@strict inside async block - ownership cannot be tracked across yield",
            Self::K0065 => "select branch may not be cancel-safe",
            Self::K0067 => "handler request-state leaks across async boundary",
            Self::K0080 => "structural ownership conflict - no automatic fix possible",
            Self::K0080P1 => "ownership pattern will require architectural decision at migration",
            Self::K0080P2 => "parent-child Rc back-pointer tree - cycle risk",
            Self::K0080P3 => "shared mutable state at 3+ call sites",
            Self::K0080P4 => "self-referential struct - infinite size without indirection",
            Self::K0081 => "ownership problem is too large to choose automatically",
            Self::K0082 => "ownership analysis took too long",
            Self::K0083 => "ownership choice requires human review",
            Self::K0084 => "ownership choice needs confirmation",
            Self::K0085 => "ownership suggestion needs manual review",
            Self::K0090 => "migration cannot continue - value crosses into external crate",
            Self::K0095 => "ownership of macro-generated value cannot be inferred",
            Self::K0096 => "legacy mode directive is a guarantee profile alias",
            Self::K0099 => "rustc error remapped to Kobo source",
            Self::K0100 => "checked scenario dropped an unresolved liveness token",
            Self::K0101 => "liveness obligation escapes local analysis",
            Self::K0102 => "raw nondeterminism appears on a replay path",
            Self::K0103 => "scenario cannot replay an uncontrolled effect",
            Self::K0104 => ".kwit replay diverged from recorded history",
            Self::K0105 => "scenario exceeded quick-profile budget",
            Self::K0106 => "witness shrink is unsafe",
            Self::K0107 => "unmodeled external crate boundary",
            Self::K0108 => "replay obligation suppressed",
            Self::K0109 => "invalid field capability view",
            Self::K0110 => "syntax error recovered",
            Self::K0111 => "unclosed delimiter",
            Self::K0112 => "invalid item skipped",
            Self::K0113 => "parser recovery limit reached",
            Self::K0114 => "malformed must_call attribute",
            Self::K0115 => "invalid kwit witness schema",
            Self::K0116 => "scenario coverage incomplete",
            Self::K0117 => "semantic trace and harness trace diverged",
            Self::K0118 => "stale or mismatched evidence artifact",
            _ => "reserved Kobo diagnostic slot",
        }
    }

    pub fn metadata(self) -> KErrorMetadata {
        if let Some(entry) = crate::diagnostic_registry().get(self) {
            return KErrorMetadata {
                short_description: entry.title,
            };
        }

        KErrorMetadata {
            short_description: self.short_description(),
        }
    }
}

/// Resolve the severity of a K-code diagnostic based on compiler guarantee policy.
///
/// Single source of truth for severity routing: registry metadata plus guarantee policy.
/// Returns `None` for diagnostics that should not be emitted in the given mode.
pub fn resolve_severity(code: KErrorCode, policy: &GuaranteePolicy) -> Option<Severity> {
    crate::diagnostic_registry()
        .get(code)
        .and_then(|entry| entry.severity_policy.resolve(policy))
}

#[cfg(test)]
mod tests {
    use super::{resolve_severity, KErrorCode, Severity};
    use kobo_ir::{GuaranteePolicy, GuaranteeProfile};

    fn policy(profile: GuaranteeProfile) -> GuaranteePolicy {
        GuaranteePolicy::for_profile(profile)
    }

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

    #[test]
    fn k0001_script_is_silent() {
        assert_eq!(
            resolve_severity(KErrorCode::K0001, &policy(GuaranteeProfile::Dev)),
            None
        );
    }

    #[test]
    fn k0001_checked_is_warning() {
        assert_eq!(
            resolve_severity(KErrorCode::K0001, &policy(GuaranteeProfile::Checked)),
            Some(Severity::Warning)
        );
    }

    #[test]
    fn k0001_strict_is_error() {
        assert_eq!(
            resolve_severity(KErrorCode::K0001, &policy(GuaranteeProfile::Release)),
            Some(Severity::Error)
        );
    }

    #[test]
    fn k0002_script_is_silent() {
        assert_eq!(
            resolve_severity(KErrorCode::K0002, &policy(GuaranteeProfile::Dev)),
            None
        );
    }

    #[test]
    fn perf_advisory_k0020_always_warning() {
        for policy in [
            policy(GuaranteeProfile::Dev),
            policy(GuaranteeProfile::Checked),
            policy(GuaranteeProfile::Release),
        ] {
            assert_eq!(
                resolve_severity(KErrorCode::K0020, &policy),
                Some(Severity::Warning)
            );
        }
    }

    #[test]
    fn strict_boundary_k0041_always_error() {
        for policy in [
            policy(GuaranteeProfile::Dev),
            policy(GuaranteeProfile::Checked),
            policy(GuaranteeProfile::Release),
        ] {
            assert_eq!(
                resolve_severity(KErrorCode::K0041, &policy),
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
                resolve_severity(code, &policy(GuaranteeProfile::Dev)),
                Some(Severity::Warning)
            );
            assert_eq!(
                resolve_severity(code, &policy(GuaranteeProfile::Checked)),
                Some(Severity::Warning)
            );
            assert_eq!(
                resolve_severity(code, &policy(GuaranteeProfile::Release)),
                Some(Severity::Error)
            );
        }
    }

    #[test]
    fn structural_advisory_precursors_always_note() {
        for code in [
            KErrorCode::K0080,
            KErrorCode::K0080P1,
            KErrorCode::K0080P2,
            KErrorCode::K0080P3,
            KErrorCode::K0080P4,
        ] {
            for policy in [
                policy(GuaranteeProfile::Dev),
                policy(GuaranteeProfile::Checked),
                policy(GuaranteeProfile::Release),
            ] {
                assert_eq!(resolve_severity(code, &policy), Some(Severity::Note));
            }
        }
    }

    #[test]
    fn rustc_remap_k0099_always_error() {
        for policy in [
            policy(GuaranteeProfile::Dev),
            policy(GuaranteeProfile::Checked),
            policy(GuaranteeProfile::Release),
        ] {
            assert_eq!(
                resolve_severity(KErrorCode::K0099, &policy),
                Some(Severity::Error)
            );
        }
    }
}
