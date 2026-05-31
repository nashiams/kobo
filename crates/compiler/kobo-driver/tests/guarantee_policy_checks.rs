use kobo_driver::{
    parse_kobo_config, CompileSession, ErrorPolicy, GuaranteeLevel, GuaranteePolicy,
    GuaranteeProfile, KoboConfig,
};
use kobo_errors::{resolve_severity, KErrorCode, Severity};

#[test]
fn compiler_config_is_owned_by_guarantee_policy() {
    let mut config = KoboConfig::default();
    assert_eq!(config.guarantee_policy.profile(), GuaranteeProfile::Dev);

    config.guarantee_policy = GuaranteePolicy::for_profile(GuaranteeProfile::Release);
    let session = CompileSession::new(config);

    assert_eq!(
        session.guarantee_policy().profile(),
        GuaranteeProfile::Release
    );

    let checked_session = CompileSession::new(KoboConfig {
        guarantee_policy: GuaranteePolicy::for_profile(GuaranteeProfile::Checked),
        ..Default::default()
    });
    assert!(checked_session.guarantee_policy().diag_always_active());
}

#[test]
fn diagnostics_resolve_from_guarantee_policy_not_legacy_mode() {
    let dev_policy = GuaranteePolicy::for_profile(GuaranteeProfile::Dev);
    let checked_policy = GuaranteePolicy::for_profile(GuaranteeProfile::Checked);
    let release_policy = GuaranteePolicy::for_profile(GuaranteeProfile::Release);

    assert_eq!(resolve_severity(KErrorCode::K0001, &dev_policy), None);
    assert_eq!(
        resolve_severity(KErrorCode::K0001, &checked_policy),
        Some(Severity::Warning)
    );
    assert_eq!(
        resolve_severity(KErrorCode::K0001, &release_policy),
        Some(Severity::Error)
    );
}

#[test]
fn release_policy_makes_liveness_and_replay_strict() {
    let release_policy = GuaranteePolicy::for_profile(GuaranteeProfile::Release);
    let guarantees = release_policy.guarantees();

    assert_eq!(guarantees.liveness(), GuaranteeLevel::Strict);
    assert_eq!(guarantees.replay(), GuaranteeLevel::Strict);
}

#[test]
fn kobo_config_expands_profile_and_guarantee_tables_into_policy() {
    let config = parse_kobo_config(
        r#"
[kobo]
profile = "release"

[guarantees]
ownership = "checked"
errors = "typed"

[profiles.release.guarantees]
boundaries = "strict"
"#,
    )
    .expect("config should parse");

    let guarantees = config.guarantee_policy.guarantees();
    assert_eq!(config.guarantee_policy.profile(), GuaranteeProfile::Release);
    assert_eq!(guarantees.ownership(), GuaranteeLevel::Checked);
    assert_eq!(guarantees.boundaries(), GuaranteeLevel::Strict);
    assert_eq!(guarantees.errors(), ErrorPolicy::Typed);
}
