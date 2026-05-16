use crate::mode_parse::{parse_file_mode, parse_legacy_mode_directive, ModeParseError};
use kobo_ir::KoboMode;

/// S-26 Contract: `//! kobo:mode = script` → Script
#[test]
fn parse_script_mode() {
    let source = "//! kobo:mode = script\nfn main() {}";
    assert_eq!(parse_file_mode(source).unwrap(), Some(KoboMode::Script));
}

/// S-26 Contract: `//! kobo:mode = checked` → Checked
#[test]
fn parse_checked_mode() {
    let source = "//! kobo:mode = checked\nfn main() {}";
    assert_eq!(parse_file_mode(source).unwrap(), Some(KoboMode::Checked));
}

/// S-26 Contract: `//! kobo:mode = strict` → Strict
#[test]
fn parse_strict_mode() {
    let source = "//! kobo:mode = strict\nfn main() {}";
    assert_eq!(parse_file_mode(source).unwrap(), Some(KoboMode::Strict));
}

#[test]
fn legacy_mode_directive_reports_equivalent_profile() {
    let source = "//! kobo:mode = strict\nfn main() {}";
    let directive = parse_legacy_mode_directive(source)
        .unwrap()
        .expect("directive should parse");
    assert_eq!(directive.line, 1);
    assert_eq!(directive.value, "strict");
    assert_eq!(directive.mode, KoboMode::Strict);
    assert_eq!(directive.profile_name(), "release");
}

/// S-26 Contract: No mode attribute → None
#[test]
fn no_mode_returns_none() {
    let source = "fn main() {}\n// some comment";
    assert_eq!(parse_file_mode(source).unwrap(), None);
}

/// S-26 Contract: Invalid mode string → error
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

/// S-26 Contract: Duplicate mode attributes → error
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

/// S-26 Contract: Mode attribute on line 10 is still parsed
#[test]
fn mode_on_line_10_is_parsed() {
    let mut source = String::new();
    for _ in 0..9 {
        source.push_str("// comment\n");
    }
    source.push_str("//! kobo:mode = strict\n");
    assert_eq!(parse_file_mode(&source).unwrap(), Some(KoboMode::Strict));
}

/// S-26 Contract: Mode attribute on line 11 is NOT parsed (beyond first 10 lines)
#[test]
fn mode_on_line_11_is_ignored() {
    let mut source = String::new();
    for _ in 0..10 {
        source.push_str("// comment\n");
    }
    source.push_str("//! kobo:mode = strict\n");
    assert_eq!(parse_file_mode(&source).unwrap(), None);
}

/// S-26 Contract: Extra whitespace around `=` is tolerated
#[test]
fn mode_with_extra_whitespace() {
    let source = "//!   kobo:mode   =   checked  \nfn main() {}";
    assert_eq!(parse_file_mode(source).unwrap(), Some(KoboMode::Checked));
}

/// S-26 Contract: Regular comments (not `//!`) are ignored
#[test]
fn regular_comment_is_ignored() {
    let source = "// kobo:mode = strict\nfn main() {}";
    assert_eq!(parse_file_mode(source).unwrap(), None);
}
