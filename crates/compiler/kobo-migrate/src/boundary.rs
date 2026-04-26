use kobo_ir::{KirNodeId, KoboSpan};
/// Detect crate boundaries where migration must stop (K0090).
///
/// K0090 fires when a binding is passed to an external crate function
/// that has a signature requiring a specific ownership type that cannot be changed.
///
/// Example:
///   external_crate::process(&data)  — migration cannot change external_crate signature
use std::collections::HashMap;

/// An external crate call site.
#[derive(Clone, Debug)]
pub struct ExternalCall {
    pub crate_name: String,
    pub function_name: String,
    pub span: KoboSpan,
    pub passed_bindings: Vec<KirNodeId>,
}

/// A K0090 boundary violation.
#[derive(Clone, Debug)]
pub struct BoundaryViolation {
    pub binding_id: KirNodeId,
    pub binding_name: String,
    pub crate_name: String,
    pub function_name: String,
    pub span: KoboSpan,
    pub message: String,
}

/// Detect external crate function calls that impose ownership constraints.
pub fn detect_crate_boundaries(external_calls: &[ExternalCall]) -> Vec<BoundaryViolation> {
    detect_crate_boundaries_with_names(external_calls, &HashMap::new())
}

/// Like `detect_crate_boundaries` but resolves `binding_name` from the provided map.
pub fn detect_crate_boundaries_with_names(
    external_calls: &[ExternalCall],
    binding_names: &HashMap<KirNodeId, String>,
) -> Vec<BoundaryViolation> {
    let mut violations = Vec::new();

    for call in external_calls {
        // Only flag if the crate is not the current crate and not std.
        if call.crate_name == "std" || call.crate_name == "core" || call.crate_name == "alloc" {
            continue;
        }

        for binding_id in &call.passed_bindings {
            let binding_name = binding_names.get(binding_id).cloned().unwrap_or_default();
            violations.push(BoundaryViolation {
                binding_id: *binding_id,
                binding_name,
                crate_name: call.crate_name.clone(),
                function_name: call.function_name.clone(),
                span: call.span,
                message: format!(
                    "binding crosses into external crate {} — migration must stop",
                    call.crate_name
                ),
            });
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::FileId;

    #[test]
    fn detect_external_crate_call() {
        let calls = vec![ExternalCall {
            crate_name: "serde_json".to_owned(),
            function_name: "from_str".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(1)],
        }];

        let violations = detect_crate_boundaries(&calls);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].crate_name, "serde_json");
    }

    #[test]
    fn skip_std_library_calls() {
        let calls = vec![ExternalCall {
            crate_name: "std".to_owned(),
            function_name: "println".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(1)],
        }];

        let violations = detect_crate_boundaries(&calls);
        assert_eq!(violations.len(), 0, "should not flag std calls");
    }

    #[test]
    fn multiple_bindings_in_one_call() {
        let calls = vec![ExternalCall {
            crate_name: "mylib".to_owned(),
            function_name: "process".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(1), KirNodeId(2), KirNodeId(3)],
        }];

        let violations = detect_crate_boundaries(&calls);
        assert_eq!(violations.len(), 3);
    }

    #[test]
    fn empty_calls_no_violations() {
        let violations = detect_crate_boundaries(&[]);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn core_and_alloc_skipped() {
        let calls = vec![
            ExternalCall {
                crate_name: "core".to_owned(),
                function_name: "mem::size_of".to_owned(),
                span: KoboSpan::new(0, 10, FileId(0)),
                passed_bindings: vec![KirNodeId(1)],
            },
            ExternalCall {
                crate_name: "alloc".to_owned(),
                function_name: "vec::from".to_owned(),
                span: KoboSpan::new(0, 10, FileId(0)),
                passed_bindings: vec![KirNodeId(2)],
            },
        ];

        let violations = detect_crate_boundaries(&calls);
        assert_eq!(violations.len(), 0, "should skip core and alloc");
    }

    // ─── BUG-10 tests: binding_name populated from map ───

    #[test]
    fn binding_name_filled_from_map() {
        let mut names = HashMap::new();
        names.insert(KirNodeId(1), "my_data".to_owned());
        names.insert(KirNodeId(2), "config".to_owned());

        let calls = vec![ExternalCall {
            crate_name: "serde_json".to_owned(),
            function_name: "to_string".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(1), KirNodeId(2)],
        }];

        let violations = detect_crate_boundaries_with_names(&calls, &names);
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].binding_name, "my_data");
        assert_eq!(violations[1].binding_name, "config");
    }

    #[test]
    fn binding_name_defaults_to_empty_if_not_in_map() {
        let names = HashMap::new(); // empty

        let calls = vec![ExternalCall {
            crate_name: "serde_json".to_owned(),
            function_name: "to_string".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(99)],
        }];

        let violations = detect_crate_boundaries_with_names(&calls, &names);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].binding_name, "",
            "unknown binding should default to empty"
        );
    }

    #[test]
    fn old_api_still_works_with_empty_names() {
        let calls = vec![ExternalCall {
            crate_name: "tokio".to_owned(),
            function_name: "spawn".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(1)],
        }];

        let violations = detect_crate_boundaries(&calls);
        assert_eq!(violations.len(), 1);
        // Old API should still produce violations (just with empty binding_name)
        assert_eq!(violations[0].binding_name, "");
    }

    // ─── v0.8 edge-case tests ───

    /// std, core, alloc crates NEVER flagged.
    #[test]
    fn std_core_alloc_never_flagged() {
        for crate_name in ["std", "core", "alloc"] {
            let calls = vec![ExternalCall {
                crate_name: crate_name.to_owned(),
                function_name: "some_fn".to_owned(),
                span: KoboSpan::new(0, 10, FileId(0)),
                passed_bindings: vec![KirNodeId(1)],
            }];
            let violations = detect_crate_boundaries(&calls);
            assert!(
                violations.is_empty(),
                "{crate_name} should never be flagged as boundary"
            );
        }
    }

    /// External crate IS flagged.
    #[test]
    fn external_crate_flagged() {
        let calls = vec![ExternalCall {
            crate_name: "rand".to_owned(),
            function_name: "random".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(5)],
        }];
        let violations = detect_crate_boundaries(&calls);
        assert!(
            !violations.is_empty(),
            "external crate 'rand' must be flagged"
        );
    }

    /// Binding name populated from map.
    #[test]
    fn binding_name_populated_from_map() {
        let mut names = HashMap::new();
        names.insert(KirNodeId(7), "my_connection".to_owned());

        let calls = vec![ExternalCall {
            crate_name: "diesel".to_owned(),
            function_name: "insert_into".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(7)],
        }];

        let violations = detect_crate_boundaries_with_names(&calls, &names);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].binding_name, "my_connection");
    }

    /// Multiple bindings per call → one violation per binding.
    #[test]
    fn multiple_bindings_per_call() {
        let calls = vec![ExternalCall {
            crate_name: "serde".to_owned(),
            function_name: "serialize".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![KirNodeId(1), KirNodeId(2), KirNodeId(3)],
        }];
        let violations = detect_crate_boundaries(&calls);
        assert_eq!(violations.len(), 3, "one violation per passed binding");
    }

    /// Empty calls → empty violations.
    #[test]
    fn empty_calls_empty_violations() {
        let violations = detect_crate_boundaries(&[]);
        assert!(violations.is_empty());
    }

    /// Multiple external calls from different crates.
    #[test]
    fn multiple_crates_all_flagged() {
        let calls = vec![
            ExternalCall {
                crate_name: "tokio".to_owned(),
                function_name: "spawn".to_owned(),
                span: KoboSpan::new(0, 10, FileId(0)),
                passed_bindings: vec![KirNodeId(1)],
            },
            ExternalCall {
                crate_name: "hyper".to_owned(),
                function_name: "client".to_owned(),
                span: KoboSpan::new(20, 30, FileId(0)),
                passed_bindings: vec![KirNodeId(2)],
            },
        ];
        let violations = detect_crate_boundaries(&calls);
        assert_eq!(violations.len(), 2);
    }

    /// Call with zero passed_bindings → zero violations.
    #[test]
    fn zero_bindings_zero_violations() {
        let calls = vec![ExternalCall {
            crate_name: "serde".to_owned(),
            function_name: "serialize".to_owned(),
            span: KoboSpan::new(0, 10, FileId(0)),
            passed_bindings: vec![],
        }];
        let violations = detect_crate_boundaries(&calls);
        assert!(violations.is_empty());
    }
}
