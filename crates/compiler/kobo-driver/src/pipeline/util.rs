use kobo_ir::{Kir, KoboMode};

/// Resolves the effective mode for a file.
///
/// Priority: CLI flag > file attribute (`//! kobo:mode = …`) > Kobo.toml [S-26].
pub fn effective_mode(
    cli_mode: Option<KoboMode>,
    file_mode: Option<KoboMode>,
    config_mode: KoboMode,
) -> KoboMode {
    cli_mode.or(file_mode).unwrap_or(config_mode)
}

/// S-21: Apply lifetime erasure to Rust source in Script/Checked mode.
///
/// Rewrites reference parameters (`&T`, `&mut T`, `&str`) to owned types
/// (`T`, `T`, `String`). Intended for Script-mode prototyping where the
/// user opts in to simplified ownership.
///
/// Call this on the codegen output when `--erase-lifetimes` is requested.
pub fn apply_lifetime_erasure(source: &str, mode: KoboMode) -> String {
    kobo_transform::lifetime_erase::rewrite_fn_signature(source, mode)
}

/// S-21: Report clone debt introduced by public-signature lifetime erasure.
pub fn lifetime_erasure_debt_report(source: &str, mode: KoboMode) -> String {
    let result = kobo_transform::lifetime_erase::analyze_source(source, mode);
    kobo_transform::lifetime_erase::format_clone_debt(&result.clone_sites)
}

/// S-17: Apply extract-before-borrow rewrites to source from production KIR facts.
pub fn extract_before_borrow_rewrite(source: &str, kir: &Kir) -> (String, usize) {
    let sites =
        kobo_transform::patterns::extract_borrow::find_extract_before_borrow(kir.transform_facts());
    if sites.is_empty() {
        return (source.to_owned(), 0);
    }
    let rewrite = kobo_transform::patterns::extract_borrow::apply_extract_before_borrow_rewrites(
        source, &sites,
    );
    (rewrite.source, rewrite.applied_sites)
}

#[cfg(test)]
mod tests {
    use super::effective_mode;
    use kobo_ir::KoboMode;

    /// S-26: CLI flag takes highest priority.
    #[test]
    fn cli_overrides_file_and_config() {
        let result = effective_mode(
            Some(KoboMode::Strict),
            Some(KoboMode::Checked),
            KoboMode::Script,
        );
        assert_eq!(result, KoboMode::Strict);
    }

    /// S-26: File attribute overrides Kobo.toml when no CLI flag.
    #[test]
    fn file_overrides_config() {
        let result = effective_mode(None, Some(KoboMode::Checked), KoboMode::Script);
        assert_eq!(result, KoboMode::Checked);
    }

    /// S-26: Config (Kobo.toml) is the fallback.
    #[test]
    fn config_is_fallback() {
        let result = effective_mode(None, None, KoboMode::Checked);
        assert_eq!(result, KoboMode::Checked);
    }

    /// S-26: CLI overrides file attribute even when both are set.
    #[test]
    fn cli_overrides_file_when_both_set() {
        let result = effective_mode(
            Some(KoboMode::Script),
            Some(KoboMode::Strict),
            KoboMode::Checked,
        );
        assert_eq!(result, KoboMode::Script);
    }

    /// S-26: No file attribute + no CLI → uses config.
    #[test]
    fn no_file_no_cli_uses_config() {
        let result = effective_mode(None, None, KoboMode::Script);
        assert_eq!(result, KoboMode::Script);
    }

    /// S-8: spawn {} generates tokio::spawn(async move { ... }).
    #[test]
    fn spawn_generates_tokio_spawn() {
        let input = r#"
async fn main() {
    let x = 42;
    spawn {
        println!("{}", x);
    };
}
"#;
        let output = crate::test_utils::compile_and_inspect(input);
        assert!(
            output.contains("tokio::spawn(async move"),
            "expected tokio::spawn(async move in output, got:\n{output}"
        );
    }

    /// S-8 Step 2.5: async main with tokio dependency gets #[tokio::main].
    #[test]
    fn async_main_gets_executor_attribute() {
        let input = r#"
async fn main() {
    println!("hello");
}
"#;
        let mut config = crate::config::KoboConfig::default();
        config
            .dependencies
            .insert("tokio".to_string(), toml::Value::String("1".into()));
        let output = crate::test_utils::compile_and_inspect_with_config(input, config);
        assert!(
            output.contains("#[tokio::main]"),
            "expected #[tokio::main] in output, got:\n{output}"
        );
    }
}
