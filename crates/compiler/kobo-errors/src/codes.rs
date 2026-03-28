use std::fmt;

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
                    Self::K0025 => "soft hint ignored - constraint conflict",
                    Self::K0030 => "resource handle moved - cannot alias file handle",
                    Self::K0041 => "cannot enter @strict block - value has active aliases",
                    Self::K0042 => "closure captures LocalOwned<T> across @strict boundary",
                    Self::K0043 => "value moved inside @strict block - cannot re-wrap on exit",
                    Self::K0060 => "RefCell borrow is live at suspend point",
                    Self::K0061 => "future requires Send but value cannot safely cross thread boundary",
                    Self::K0062 => "Mutex guard would live across .await",
                    Self::K0063 => "@strict block inside async fn without @strict async fn",
                    Self::K0080 => "structural ownership conflict - no automatic fix possible",
                    Self::K0080P1 => "ownership pattern will require architectural decision at migration",
                    Self::K0081 => "ownership cluster too large for automatic solving",
                    Self::K0082 => "solver exceeded its time budget",
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

#[cfg(test)]
mod tests {
    use super::{KErrorCode, Severity};

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
}
