use std::fmt;

use kobo_ir::KoboSpan;

use crate::codes::{KErrorCode, Severity};

/// Typed diagnostic spec used as the long-term source of truth for renderer data.
pub trait DiagnosticSpec: Copy + Clone + fmt::Debug + Eq + PartialEq {
    fn code(self) -> KErrorCode;
    fn short_description(self) -> &'static str;
}

/// Stub renderer-facing marker trait until the v0.2 diagnostic renderer lands.
pub trait RendererDiagnostic: DiagnosticSpec {}

impl<T> RendererDiagnostic for T where T: DiagnosticSpec {}

macro_rules! define_diagnostic_specs {
    ($( $code:ident => $description:literal, )* ) => {
        $(
            #[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
            pub struct $code;

            impl $code {
                pub const CODE: KErrorCode = KErrorCode::$code;
                pub const DESCRIPTION: &'static str = $description;
            }

            impl DiagnosticSpec for $code {
                fn code(self) -> KErrorCode {
                    Self::CODE
                }

                fn short_description(self) -> &'static str {
                    Self::DESCRIPTION
                }
            }
        )*

        /// Temporary shim enum kept for the v0.1 formatter surface.
        #[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
        pub enum DiagMessage {
            $( $code, )*
        }

        $(
            impl From<$code> for DiagMessage {
                fn from(_: $code) -> Self {
                    Self::$code
                }
            }
        )*

        impl DiagMessage {
            pub const fn code(self) -> KErrorCode {
                match self {
                    $( Self::$code => $code::CODE, )*
                }
            }

            pub const fn short_description(self) -> &'static str {
                match self {
                    $( Self::$code => $code::DESCRIPTION, )*
                }
            }
        }

        impl fmt::Display for DiagMessage {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.short_description())
            }
        }

        impl DiagnosticSpec for DiagMessage {
            fn code(self) -> KErrorCode {
                DiagMessage::code(self)
            }

            fn short_description(self) -> &'static str {
                DiagMessage::short_description(self)
            }
        }
    };
}

define_diagnostic_specs! {
    K0001 => "value used after move",
    K0002 => "cannot borrow as mutable - already borrowed",
    K0003 => "ownership diagnostic stub",
    K0004 => "ownership diagnostic stub",
    K0005 => "ownership diagnostic stub",
    K0006 => "ownership diagnostic stub",
    K0007 => "ownership diagnostic stub",
    K0008 => "ownership diagnostic stub",
    K0009 => "ownership diagnostic stub",
    K0010 => "ownership diagnostic stub",
    K0011 => "ownership diagnostic stub",
    K0012 => "ownership diagnostic stub",
    K0013 => "ownership diagnostic stub",
    K0014 => "ownership diagnostic stub",
    K0015 => "ownership diagnostic stub",
    K0016 => "ownership diagnostic stub",
    K0017 => "ownership diagnostic stub",
    K0018 => "ownership diagnostic stub",
    K0019 => "ownership diagnostic stub",
    K0020 => "RefCell accessed >10,000 times in hot path",
    K0021 => "performance diagnostic stub",
    K0022 => "performance diagnostic stub",
    K0023 => "performance diagnostic stub",
    K0024 => "performance diagnostic stub",
    K0025 => "soft hint ignored - constraint conflict",
    K0026 => "performance diagnostic stub",
    K0027 => "performance diagnostic stub",
    K0028 => "performance diagnostic stub",
    K0029 => "performance diagnostic stub",
    K0030 => "resource handle moved - cannot alias file handle",
    K0031 => "performance diagnostic stub",
    K0032 => "performance diagnostic stub",
    K0033 => "performance diagnostic stub",
    K0034 => "performance diagnostic stub",
    K0035 => "performance diagnostic stub",
    K0036 => "performance diagnostic stub",
    K0037 => "performance diagnostic stub",
    K0038 => "performance diagnostic stub",
    K0039 => "performance diagnostic stub",
    K0040 => "strict-boundary diagnostic stub",
    K0041 => "cannot enter @strict block - value has active aliases",
    K0042 => "closure captures LocalOwned<T> across @strict boundary",
    K0043 => "value moved inside @strict block - cannot re-wrap on exit",
    K0044 => "strict-boundary diagnostic stub",
    K0045 => "strict-boundary diagnostic stub",
    K0046 => "strict-boundary diagnostic stub",
    K0047 => "strict-boundary diagnostic stub",
    K0048 => "strict-boundary diagnostic stub",
    K0049 => "strict-boundary diagnostic stub",
    K0050 => "strict-boundary diagnostic stub",
    K0051 => "strict-boundary diagnostic stub",
    K0052 => "strict-boundary diagnostic stub",
    K0053 => "strict-boundary diagnostic stub",
    K0054 => "strict-boundary diagnostic stub",
    K0055 => "strict-boundary diagnostic stub",
    K0056 => "strict-boundary diagnostic stub",
    K0057 => "strict-boundary diagnostic stub",
    K0058 => "strict-boundary diagnostic stub",
    K0059 => "strict-boundary diagnostic stub",
    K0060 => "RefCell borrow is live at suspend point",
    K0061 => "future requires Send but value cannot safely cross thread boundary",
    K0062 => "Mutex guard would live across .await",
    K0063 => "@strict block inside async fn without @strict async fn",
    K0064 => "async ownership diagnostic stub",
    K0065 => "async ownership diagnostic stub",
    K0066 => "async ownership diagnostic stub",
    K0067 => "async ownership diagnostic stub",
    K0068 => "async ownership diagnostic stub",
    K0069 => "async ownership diagnostic stub",
    K0070 => "async ownership diagnostic stub",
    K0071 => "async ownership diagnostic stub",
    K0072 => "async ownership diagnostic stub",
    K0073 => "async ownership diagnostic stub",
    K0074 => "async ownership diagnostic stub",
    K0075 => "async ownership diagnostic stub",
    K0076 => "async ownership diagnostic stub",
    K0077 => "async ownership diagnostic stub",
    K0078 => "async ownership diagnostic stub",
    K0079 => "async ownership diagnostic stub",
    K0080 => "structural ownership conflict - no automatic fix possible",
    K0080P1 => "ownership pattern will require architectural decision at migration",
    K0080P2 => "structural precursor advisory stub",
    K0080P3 => "structural precursor advisory stub",
    K0080P4 => "structural precursor advisory stub",
    K0081 => "ownership cluster too large for automatic solving",
    K0082 => "solver exceeded its time budget",
    K0083 => "structural conflict diagnostic stub",
    K0084 => "structural conflict diagnostic stub",
    K0085 => "structural conflict diagnostic stub",
    K0086 => "structural conflict diagnostic stub",
    K0087 => "structural conflict diagnostic stub",
    K0088 => "structural conflict diagnostic stub",
    K0089 => "structural conflict diagnostic stub",
    K0090 => "migration cannot continue - value crosses into external crate",
    K0091 => "structural conflict diagnostic stub",
    K0092 => "structural conflict diagnostic stub",
    K0093 => "structural conflict diagnostic stub",
    K0094 => "structural conflict diagnostic stub",
    K0095 => "ownership of macro-generated value cannot be inferred",
    K0096 => "structural conflict diagnostic stub",
    K0097 => "structural conflict diagnostic stub",
    K0098 => "structural conflict diagnostic stub",
    K0099 => "structural conflict diagnostic stub",
}

/// What Kobo did or refused at this site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagDecision(pub String);

/// Concrete next step for the user.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagHelp(pub String);

/// Suggested CLI command for immediate follow-up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliSuggestion(pub String);

/// Structured diagnostic payload carried through the pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KDiagnostic {
    pub code: KErrorCode,
    pub severity: Severity,
    pub primary_span: KoboSpan,
    pub message: DiagMessage,
    pub decision: DiagDecision,
    pub help: Option<DiagHelp>,
    pub run: Option<CliSuggestion>,
}

impl DiagDecision {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl DiagHelp {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl CliSuggestion {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for DiagDecision {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DiagDecision {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DiagHelp {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DiagHelp {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for CliSuggestion {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for CliSuggestion {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for DiagDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DiagHelp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for CliSuggestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl KDiagnostic {
    pub fn new<T>(
        message: T,
        severity: Severity,
        primary_span: KoboSpan,
        decision: impl Into<DiagDecision>,
    ) -> Self
    where
        T: DiagnosticSpec + Into<DiagMessage>,
    {
        Self {
            code: message.code(),
            severity,
            primary_span,
            message: message.into(),
            decision: decision.into(),
            help: None,
            run: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<DiagHelp>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_run(mut self, run: impl Into<CliSuggestion>) -> Self {
        self.run = Some(run.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, KoboSpan};

    use crate::codes::{KErrorCode, Severity};

    use super::{DiagMessage, DiagnosticSpec, KDiagnostic, K0001, K0060};

    #[test]
    fn typed_spec_maps_back_to_code() {
        assert_eq!(K0060.code(), KErrorCode::K0060);
    }

    #[test]
    fn enum_shim_still_maps_back_to_code() {
        assert_eq!(DiagMessage::K0060.code(), KErrorCode::K0060);
    }

    #[test]
    fn diagnostic_builder_derives_code_from_typed_spec() {
        let diagnostic = KDiagnostic::new(
            K0001,
            Severity::Error,
            KoboSpan::new(3, 8, FileId(0)),
            "inserted clone at move site",
        );

        assert_eq!(diagnostic.code, KErrorCode::K0001);
    }
}
