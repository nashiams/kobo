use super::ExplainProse;

pub(super) fn ecosystem_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0120 => ExplainProse {
            fix: "- Fix the malformed [ecosystem] or boundary policy entry in Kobo.toml.\n\
                  - Replace the invalid policy with typed, model, record, activity, outside, opaque, or debt.\n\
                  - Rerun `kobo doctor --deps --json` before trusting project-wide policy.",
            example: "Problem:\n[ecosystem.policy]\nserde_json = \"exact\"\n\n\
                      Fix:\nUse a supported policy such as typed, record, activity, opaque, or debt, then rerun doctor.",
        },
        crate::KErrorCode::K0121 => ExplainProse {
            fix: "- Add the missing schema, crate, version, effect, or obligation field.\n\
                  - Regenerate the declaration from source and review the generated questions.\n\
                  - Remove the declaration from Kobo.toml until it validates.",
            example: "Problem:\nA kobo.d.toml file names a crate but omits effect metadata for a typed boundary.\n\n\
                      Fix:\nFill in the declaration fields or regenerate the declaration before enabling typed policy.",
        },
        crate::KErrorCode::K0122 => ExplainProse {
            fix: "- Add a matching crate.kobo.d.toml, kobo.d.toml, or validated kobo-types package.\n\
                  - Change the boundary to record, activity, opaque, or debt when typed metadata is not available.\n\
                  - Point Kobo.toml at the declaration package that matches the crate version.",
            example: "Problem:\n#[kobo::boundary(policy = \"typed\")]\nreqwest::get(url).await?;\n\n\
                      Fix:\nInstall or configure a validated reqwest declaration, or choose a non-typed boundary policy.",
        },
        crate::KErrorCode::K0123 => ExplainProse {
            fix: "- Install the adapter package named by Kobo.toml.\n\
                  - Update the adapter so its crate, version, and trust metadata match the dependency.\n\
                  - Downgrade the boundary to record, activity, opaque, or debt until the adapter validates.",
            example: "Problem:\nKobo.toml selects kobo-adapter-sqlx, but the adapter is missing or declares a different sqlx version.\n\n\
                      Fix:\nInstall a compatible adapter package or choose a policy that does not require that adapter.",
        },
        crate::KErrorCode::K0124 => ExplainProse {
            fix: "- Regenerate the witness with recording enabled for this boundary.\n\
                  - Attach the recorded event/result evidence to the replay artifact.\n\
                  - Change the boundary policy when the effect should not be recorded.",
            example: "Problem:\nA record boundary calls a payment API, but the witness has no recorded request or result entry.\n\n\
                      Fix:\nRerun the scenario with recording or use a policy that matches the available evidence.",
        },
        crate::KErrorCode::K0125 => ExplainProse {
            fix: "- Add retry and idempotency metadata to the activity declaration.\n\
                  - Mark the result and compensation behavior so replay review can reason about retries.\n\
                  - Use record, opaque, or debt until the activity policy is complete.",
            example: "Problem:\nAn activity boundary sends an email but does not say whether retrying is idempotent.\n\n\
                      Fix:\nDeclare retry, idempotency, result, and compensation metadata before claiming activity evidence.",
        },
        crate::KErrorCode::K0126 => ExplainProse {
            fix: "- Rebuild the upstream Kobo package that produced the .kobo-summary.\n\
                  - Update Kobo.toml with the new summary hash after reviewing the producer artifact.\n\
                  - Remove stale summaries before running downstream exact replay checks.",
            example: "Problem:\nKobo.toml pins a .kobo-summary hash, but the file body hashes to a different value.\n\n\
                      Fix:\nRegenerate the summary and update the pinned hash only after the producer package is rebuilt.",
        },
        crate::KErrorCode::K0127 => ExplainProse {
            fix: "- Review every generated bindgen question and complete missing effect metadata.\n\
                  - Rerun bindgen with `--path` for source-backed extraction when a registry seed was used.\n\
                  - Keep the declaration out of typed policy until review-required fields are resolved.",
            example: "Problem:\nkobo bindgen sqlx creates a declaration with review_required = true.\n\n\
                      Fix:\nReview the generated API effects or rerun bindgen against a source path before trusting typed replay.",
        },
        crate::KErrorCode::K0128 => ExplainProse {
            fix: "- Preserve Cargo dependency, feature, target, dev, and build metadata exactly.\n\
                  - Report an explicit compatibility error when Kobo cannot migrate that Cargo shape.\n\
                  - Rerun Cargo after migration and compare package metadata before accepting the change.",
            example: "Problem:\nA target-specific Cargo dependency is migrated as an unconditional Kobo dependency.\n\n\
                      Fix:\nKeep the target condition or stop with a compatibility diagnostic instead of changing build behavior.",
        },
        crate::KErrorCode::K0129 => ExplainProse {
            fix: "- Keep the witness partial when external crate internals were not inspected.\n\
                  - Add matching declaration, adapter, summary, and trace evidence before claiming exact replay.\n\
                  - Regenerate exact evidence only after semantic and harness traces agree.",
            example: "Problem:\nA replay run claims exact coverage for a reqwest call without declaration or adapter evidence.\n\n\
                      Fix:\nDowngrade to partial replay or add the missing boundary metadata and matching trace evidence.",
        },
        _ => return None,
    };

    Some(prose)
}

pub(super) fn parser_explain_prose(code: crate::KErrorCode) -> Option<ExplainProse> {
    let prose = match code {
        crate::KErrorCode::K0110 => ExplainProse {
            fix: "- Fix the highlighted syntax error before acting on later messages.\n\
                  - Add the missing delimiter or keyword shown in the snippet.\n\
                  - Rerun Kobo after this first syntax issue is gone.",
            example: "Problem:\nA missing } makes the next function look wrong too.\n\n\
                      Fix:\nAdd the missing brace first, then rerun the check.",
        },
        crate::KErrorCode::K0111 => ExplainProse {
            fix: "- Close the delimiter that Kobo points to.\n\
                  - Check the matching opening delimiter before changing later code.\n\
                  - Rerun Kobo before reviewing follow-up messages.",
            example: "Problem:\nA function call opens ( but never closes it.\n\n\
                      Fix:\nAdd the missing ) at the location Kobo highlights.",
        },
        crate::KErrorCode::K0112 => ExplainProse {
            fix: "- Fix the skipped item before checking later items.\n\
                  - Add the missing keyword, delimiter, or separator in the snippet.\n\
                  - Split the item into smaller pieces if the intended shape is unclear.",
            example: "Problem:\nA malformed function item causes Kobo to skip to the next top-level item.\n\n\
                      Fix:\nRepair that function signature first, then rerun the check.",
        },
        crate::KErrorCode::K0113 => ExplainProse {
            fix: "- Fix the earliest syntax error in the file.\n\
                  - Stop reviewing follow-up diagnostics until recovery stays under the limit.\n\
                  - Split a very broken file into smaller edits and rerun Kobo after each edit.",
            example: "Problem:\nSeveral missing delimiters make Kobo stop syntax recovery for this file.\n\n\
                      Fix:\nRepair the earliest broken delimiter and rerun Kobo.",
        },
        _ => return None,
    };

    Some(prose)
}
