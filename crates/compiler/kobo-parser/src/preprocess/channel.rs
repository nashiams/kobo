/// Parse `chan<T>` type and generate channel creation.
///
/// Input:
///   let (tx, rx) = chan<String>();
///
/// Output:
///   let (tx, rx) = tokio::sync::mpsc::channel::<String>(100);
///
/// The buffer size default is 100 (configurable via `channel_buffer_size`
/// in `Kobo.toml`).
///
/// Sender cloning into spawn blocks is handled by the auto-clone injector
/// (Phase 3). This module only rewrites the `chan<T>()` type/constructor.

/// Information about one `chan<T>()` occurrence found during preprocessing.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelInfo {
    /// The type parameter (e.g. "String", "i32", "MyMsg").
    pub type_param: String,
    /// Byte offset of the `chan<T>()` call in the original source.
    pub offset: usize,
    /// Buffer size used for the channel.
    pub buffer_size: usize,
}

/// Warnings from channel preprocessing.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelWarning {
    /// chan<T>() was used but `tokio` is not listed in dependencies.
    MissingTokioDependency {
        /// Number of chan<T>() occurrences found.
        count: usize,
    },
}

impl std::fmt::Display for ChannelWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelWarning::MissingTokioDependency { count } => {
                write!(
                    f,
                    "warning: {count} `chan<T>()` occurrence(s) emit `tokio::sync::mpsc::channel` \
                     but `tokio` is not listed as a dependency — add `tokio` with feature `sync` \
                     to Cargo.toml"
                )
            }
        }
    }
}

/// Validate that tokio dependency is present when chan<T> was used.
///
/// `dependencies` is the list of dependency names from Cargo.toml.
pub fn validate_channel_dependencies(
    infos: &[ChannelInfo],
    dependencies: &[&str],
) -> Vec<ChannelWarning> {
    if infos.is_empty() {
        return Vec::new();
    }
    let has_tokio = dependencies.iter().any(|d| *d == "tokio");
    if has_tokio {
        return Vec::new();
    }
    vec![ChannelWarning::MissingTokioDependency {
        count: infos.len(),
    }]
}

/// Rewrite all `chan<T>()` occurrences to `tokio::sync::mpsc::channel::<T>(buffer_size)`.
///
/// Returns `(rewritten_source, channel_infos)`.
pub fn preprocess_chan_type(source: &str, buffer_size: usize) -> (String, Vec<ChannelInfo>) {
    let mut result = String::with_capacity(source.len());
    let mut infos = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // Look for "chan<"
        if pos + 5 <= len && &source[pos..pos + 4] == "chan" && bytes[pos + 4] == b'<' {
            // Ensure it's not part of a longer identifier
            if pos > 0 && is_ident_char(bytes[pos - 1]) {
                result.push(bytes[pos] as char);
                pos += 1;
                continue;
            }

            // Find closing '>'
            if let Some(close_angle) = find_matching_angle_bracket(source, pos + 4) {
                let type_param = source[pos + 5..close_angle].trim().to_string();

                // Check for () after the >
                let after_close = close_angle + 1;
                if after_close + 1 < len
                    && bytes[after_close] == b'('
                    && bytes[after_close + 1] == b')'
                {
                    // Full match: chan<T>()
                    infos.push(ChannelInfo {
                        type_param: type_param.clone(),
                        offset: pos,
                        buffer_size,
                    });
                    result.push_str(&format!(
                        "tokio::sync::mpsc::channel::<{type_param}>({buffer_size})"
                    ));
                    pos = after_close + 2; // skip past ()
                    continue;
                }
            }
        }

        result.push(bytes[pos] as char);
        pos += 1;
    }

    (result, infos)
}

/// Find the matching `>` for a `<` at `start`, handling nested angle brackets.
fn find_matching_angle_bracket(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut depth = 0u32;
    let mut pos = start;

    while pos < len {
        match bytes[pos] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            _ => {}
        }
        pos += 1;
    }
    None
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chan_generates_mpsc() {
        let input = r#"let (tx, rx) = chan<String>();"#;
        let (output, infos) = preprocess_chan_type(input, 100);
        assert!(
            output.contains("tokio::sync::mpsc::channel"),
            "output: {output}"
        );
        assert!(output.contains("::<String>(100)"), "output: {output}");
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].type_param, "String");
        assert_eq!(infos[0].buffer_size, 100);
    }

    #[test]
    fn chan_with_custom_buffer_size() {
        let input = r#"let (tx, rx) = chan<i32>();"#;
        let (output, infos) = preprocess_chan_type(input, 256);
        assert!(output.contains("::<i32>(256)"), "output: {output}");
        assert_eq!(infos[0].buffer_size, 256);
    }

    #[test]
    fn chan_preserves_other_code() {
        let input = r#"let x = 42;
let (tx, rx) = chan<String>();
println!("{}", x);"#;
        let (output, _) = preprocess_chan_type(input, 100);
        assert!(output.contains("let x = 42;"), "output: {output}");
        assert!(output.contains("println!"), "output: {output}");
        assert!(
            output.contains("tokio::sync::mpsc::channel"),
            "output: {output}"
        );
    }

    #[test]
    fn chan_no_rewrite_without_parens() {
        // chan<T> without () should not be rewritten
        let input = r#"type MyChan = chan<String>;"#;
        let (output, infos) = preprocess_chan_type(input, 100);
        assert!(!output.contains("tokio::sync::mpsc"), "output: {output}");
        assert!(infos.is_empty());
    }

    #[test]
    fn chan_not_part_of_identifier() {
        // "mechan<ics>" should not match
        let input = r#"let mechanics = mechan<ics>();"#;
        let (output, infos) = preprocess_chan_type(input, 100);
        assert!(!output.contains("tokio::sync::mpsc"), "output: {output}");
        assert!(infos.is_empty());
    }

    #[test]
    fn chan_multiple_occurrences() {
        let input = r#"let (tx1, rx1) = chan<String>();
let (tx2, rx2) = chan<u64>();"#;
        let (output, infos) = preprocess_chan_type(input, 100);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].type_param, "String");
        assert_eq!(infos[1].type_param, "u64");
        assert!(output.contains("::<String>(100)"), "output: {output}");
        assert!(output.contains("::<u64>(100)"), "output: {output}");
    }

    #[test]
    fn chan_nested_type_param() {
        let input = r#"let (tx, rx) = chan<Vec<String>>();"#;
        let (output, infos) = preprocess_chan_type(input, 100);
        assert!(
            output.contains("::<Vec<String>>(100)"),
            "output: {output}"
        );
        assert_eq!(infos[0].type_param, "Vec<String>");
    }

    // ─── BUG-05 tests: tokio dependency validation ───

    #[test]
    fn channel_warning_when_tokio_missing() {
        let infos = vec![ChannelInfo {
            type_param: "String".to_owned(),
            offset: 0,
            buffer_size: 100,
        }];
        let warnings = validate_channel_dependencies(&infos, &["serde", "rand"]);
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0],
            ChannelWarning::MissingTokioDependency { count: 1 }
        ));
    }

    #[test]
    fn no_channel_warning_when_tokio_present() {
        let infos = vec![ChannelInfo {
            type_param: "String".to_owned(),
            offset: 0,
            buffer_size: 100,
        }];
        let warnings = validate_channel_dependencies(&infos, &["tokio", "serde"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn no_channel_warning_when_no_channels() {
        let warnings = validate_channel_dependencies(&[], &["serde"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn channel_warning_display() {
        let w = ChannelWarning::MissingTokioDependency { count: 3 };
        let msg = format!("{w}");
        assert!(msg.contains("tokio"), "msg: {msg}");
        assert!(msg.contains("3"), "msg: {msg}");
    }

    // ─── v0.8 edge-case tests ───

    /// chan<Vec<i32>>() with nested generics → properly rewritten.
    #[test]
    fn chan_with_nested_generics() {
        let (output, infos) = preprocess_chan_type("let (tx, rx) = chan<Vec<i32>>();", 100);
        assert!(!output.contains("chan<"), "chan< should be rewritten: {output}");
        assert_eq!(infos.len(), 1);
        assert!(infos[0].type_param.contains("Vec"));
    }

    /// "mechanics()" should NOT trigger chan rewrite.
    #[test]
    fn mechanics_not_rewritten() {
        let (output, infos) = preprocess_chan_type("let x = mechanics(42);", 100);
        assert_eq!(output, "let x = mechanics(42);");
        assert!(infos.is_empty());
    }

    /// "chan" without parens (type alias position) should NOT be rewritten.
    #[test]
    fn chan_without_parens_not_rewritten() {
        let (output, infos) = preprocess_chan_type("type MyChan = chan;", 100);
        assert!(infos.is_empty(), "bare 'chan' with no angle brackets + parens");
        // Output should contain chan unchanged
        assert!(output.contains("chan"));
    }

    /// Multiple chan declarations in one source.
    #[test]
    fn multiple_chan_declarations() {
        let source = "let (a_tx, a_rx) = chan<u8>();\nlet (b_tx, b_rx) = chan<String>();";
        let (output, infos) = preprocess_chan_type(source, 100);
        assert_eq!(infos.len(), 2);
        assert!(!output.contains("chan<u8>"), "first chan not rewritten: {output}");
        assert!(!output.contains("chan<String>"), "second chan not rewritten: {output}");
    }

    /// chan + tokio dep → no warning.
    #[test]
    fn chan_plus_tokio_no_warning() {
        let infos = vec![ChannelInfo {
            type_param: "u32".to_owned(),
            offset: 0,
            buffer_size: 50,
        }];
        let warnings = validate_channel_dependencies(&infos, &["tokio"]);
        assert!(warnings.is_empty());
    }

    /// Zero channels → zero warnings regardless of deps.
    #[test]
    fn zero_channels_zero_warnings() {
        let warnings = validate_channel_dependencies(&[], &[]);
        assert!(warnings.is_empty());
    }

    /// Multiple channels + missing tokio → count matches channel count.
    #[test]
    fn multiple_channels_warning_count() {
        let infos = vec![
            ChannelInfo { type_param: "u8".to_owned(), offset: 0, buffer_size: 10 },
            ChannelInfo { type_param: "u16".to_owned(), offset: 20, buffer_size: 10 },
            ChannelInfo { type_param: "u32".to_owned(), offset: 40, buffer_size: 10 },
        ];
        let warnings = validate_channel_dependencies(&infos, &["serde"]);
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            ChannelWarning::MissingTokioDependency { count } => assert_eq!(*count, 3),
        }
    }

    /// Empty input → no channels.
    #[test]
    fn empty_input_no_channels() {
        let (output, infos) = preprocess_chan_type("", 100);
        assert_eq!(output, "");
        assert!(infos.is_empty());
    }

    /// Buffer size from parameter, not from source.
    #[test]
    fn buffer_size_from_param() {
        let (_, infos) = preprocess_chan_type("let (tx, rx) = chan<bool>();", 42);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].buffer_size, 42);
    }
}
