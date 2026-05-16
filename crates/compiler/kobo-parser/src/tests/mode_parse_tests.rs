use crate::mode_parse::{parse_file_mode, parse_legacy_mode_directive, ModeParseError};
use kobo_ir::{GuaranteeProfile, LegacyMode};

#[test]
fn parse_script_mode_as_dev_profile() {
    let source = "//! kobo:mode = script\nfn main() {}";
    assert_eq!(
        parse_file_mode(source).unwrap(),
        Some(GuaranteeProfile::Dev)
    );
}

#[test]
fn parse_checked_mode_as_checked_profile() {
    let source = "//! kobo:mode = checked\nfn main() {}";
    assert_eq!(
        parse_file_mode(source).unwrap(),
        Some(GuaranteeProfile::Checked)
    );
}

#[test]
fn parse_strict_mode_as_release_profile() {
    let source = "//! kobo:mode = strict\nfn main() {}";
    assert_eq!(
        parse_file_mode(source).unwrap(),
        Some(GuaranteeProfile::Release)
    );
}

#[test]
fn legacy_mode_directive_reports_equivalent_profile() {
    let source = "//! kobo:mode = strict\nfn main() {}";
    let directive = parse_legacy_mode_directive(source)
        .unwrap()
        .expect("directive should parse");
    assert_eq!(directive.line, 1);
    assert_eq!(directive.value, "strict");
    assert_eq!(directive.legacy_mode, LegacyMode::Strict);
    assert_eq!(directive.profile, GuaranteeProfile::Release);
    assert_eq!(directive.profile_name(), "release");
}

#[test]
fn no_mode_returns_none() {
    let source = "fn main() {}\n// some comment";
    assert_eq!(parse_file_mode(source).unwrap(), None);
}

#[test]
fn invalid_mode_returns_error() {
    let source = "//! kobo:mode = banana\nfn main() {}";
    let err = parse_file_mode(source).unwrap_err();
    assert_eq!(
        err,
        ModeParseError::InvalidMode {
            line: 1,
            value: "banana".to_owned(),
        }
    );
}

#[test]
fn duplicate_mode_returns_error() {
    let source = "//! kobo:mode = script\n//! kobo:mode = checked\nfn main() {}";
    let err = parse_file_mode(source).unwrap_err();
    assert_eq!(
        err,
        ModeParseError::DuplicateMode {
            first_line: 1,
            second_line: 2,
        }
    );
}

#[test]
fn mode_on_line_10_is_parsed() {
    let mut source = String::new();
    for _ in 0..9 {
        source.push_str("// comment\n");
    }
    source.push_str("//! kobo:mode = strict\n");
    assert_eq!(
        parse_file_mode(&source).unwrap(),
        Some(GuaranteeProfile::Release)
    );
}

#[test]
fn mode_on_line_11_is_ignored() {
    let mut source = String::new();
    for _ in 0..10 {
        source.push_str("// comment\n");
    }
    source.push_str("//! kobo:mode = strict\n");
    assert_eq!(parse_file_mode(&source).unwrap(), None);
}

#[test]
fn mode_with_extra_whitespace() {
    let source = "//!   kobo:mode   =   checked  \nfn main() {}";
    assert_eq!(
        parse_file_mode(source).unwrap(),
        Some(GuaranteeProfile::Checked)
    );
}

#[test]
fn regular_comment_is_ignored() {
    let source = "// kobo:mode = strict\nfn main() {}";
    assert_eq!(parse_file_mode(source).unwrap(), None);
}
