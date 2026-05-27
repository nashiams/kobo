use kobo_ir::{GuaranteePolicy, Kir};

pub fn effective_guarantee_policy(
    cli_policy: Option<GuaranteePolicy>,
    file_policy: Option<GuaranteePolicy>,
    config_policy: GuaranteePolicy,
) -> GuaranteePolicy {
    cli_policy.or(file_policy).unwrap_or(config_policy)
}

/// S-21: Apply lifetime erasure to Rust source in dev/checked profiles.
pub fn apply_lifetime_erasure(source: &str, policy: &GuaranteePolicy) -> String {
    kobo_transform::lifetime_erase::rewrite_fn_signature(source, policy)
}

/// S-21: Report clone debt introduced by public-signature lifetime erasure.
pub fn lifetime_erasure_debt_report(source: &str, policy: &GuaranteePolicy) -> String {
    let result = kobo_transform::lifetime_erase::analyze_source(source, policy);
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
    use super::effective_guarantee_policy;
    use kobo_ir::{GuaranteePolicy, GuaranteeProfile};

    #[test]
    fn cli_overrides_file_and_config() {
        let result = effective_guarantee_policy(
            Some(GuaranteePolicy::for_profile(GuaranteeProfile::Release)),
            Some(GuaranteePolicy::for_profile(GuaranteeProfile::Checked)),
            GuaranteePolicy::for_profile(GuaranteeProfile::Dev),
        );
        assert_eq!(result.profile(), GuaranteeProfile::Release);
    }

    #[test]
    fn file_overrides_config() {
        let result = effective_guarantee_policy(
            None,
            Some(GuaranteePolicy::for_profile(GuaranteeProfile::Checked)),
            GuaranteePolicy::for_profile(GuaranteeProfile::Dev),
        );
        assert_eq!(result.profile(), GuaranteeProfile::Checked);
    }

    #[test]
    fn config_is_fallback() {
        let result = effective_guarantee_policy(
            None,
            None,
            GuaranteePolicy::for_profile(GuaranteeProfile::Checked),
        );
        assert_eq!(result.profile(), GuaranteeProfile::Checked);
    }

    #[test]
    fn cli_overrides_file_when_both_set() {
        let result = effective_guarantee_policy(
            Some(GuaranteePolicy::for_profile(GuaranteeProfile::Dev)),
            Some(GuaranteePolicy::for_profile(GuaranteeProfile::Release)),
            GuaranteePolicy::for_profile(GuaranteeProfile::Checked),
        );
        assert_eq!(result.profile(), GuaranteeProfile::Dev);
    }

    #[test]
    fn no_file_no_cli_uses_config() {
        let result = effective_guarantee_policy(
            None,
            None,
            GuaranteePolicy::for_profile(GuaranteeProfile::Dev),
        );
        assert_eq!(result.profile(), GuaranteeProfile::Dev);
    }

    /// S-8: spawn {} generates tokio::spawn(async move {... }).
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
