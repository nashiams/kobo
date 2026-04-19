/// Validate middleware ordering in async pipelines.
///
/// Detects common middleware ordering issues:
/// 1. Auth middleware after business logic
/// 2. Logging middleware after response
/// 3. Rate limiter after handler
///
/// This is heuristic-based: looks for function name patterns
/// (auth, log, rate_limit, etc.) and their call order.
/// Not guaranteed to catch all issues — conservative safety bias.

use kobo_ir::KoboSpan;

/// Kinds of pipeline ordering issues.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineIssue {
    /// Auth middleware placed after business-logic handler.
    AuthAfterHandler,
    /// Logging middleware placed after response.
    LogAfterResponse,
    /// Rate limiter placed after handler.
    RateLimitAfterHandler,
}

impl PipelineIssue {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AuthAfterHandler => "S14-01",
            Self::LogAfterResponse => "S14-02",
            Self::RateLimitAfterHandler => "S14-03",
        }
    }
}

/// A warning about a pipeline ordering issue.
#[derive(Clone, Debug)]
pub struct PipelineWarning {
    pub kind: PipelineIssue,
    pub span: KoboSpan,
    pub suggestion: String,
}

/// Heuristic classification of a function call's role.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CallRole {
    /// Security / auth check.
    Auth,
    /// Rate limiting.
    RateLimit,
    /// Business logic / handler.
    Handler,
    /// Logging / tracing.
    Logging,
    /// Response construction.
    Response,
    /// Unknown / unclassifiable.
    Unknown,
}

fn classify_call(name: &str) -> CallRole {
    let lower = name.to_ascii_lowercase();
    if lower.contains("auth") || lower.contains("verify") || lower.contains("check_token") {
        CallRole::Auth
    } else if lower.contains("rate_limit") || lower.contains("throttle") {
        CallRole::RateLimit
    } else if lower.contains("handle") || lower.contains("process") || lower.contains("dispatch")
    {
        CallRole::Handler
    } else if lower.contains("log") || lower.contains("trace") || lower.contains("record") {
        CallRole::Logging
    } else if lower.contains("respond") || lower.contains("response") || lower.contains("reply") {
        CallRole::Response
    } else {
        CallRole::Unknown
    }
}

/// A single call site extracted from the pipeline body.
struct PipelineCall {
    name: String,
    role: CallRole,
    /// Byte offset within the source for span construction.
    #[allow(dead_code)]
    offset: u32,
}

/// Extract function calls from a function body using syn.
///
/// We parse the items and look at top-level expressions in async fn bodies.
fn extract_pipeline_calls(items: &[syn::Item]) -> Vec<PipelineCall> {
    use syn::visit::Visit;

    struct BodyCallVisitor {
        calls: Vec<PipelineCall>,
    }

    impl<'ast> syn::visit::Visit<'ast> for BodyCallVisitor {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(ref path) = *node.func {
                if let Some(seg) = path.path.segments.last() {
                    let name = seg.ident.to_string();
                    let role = classify_call(&name);
                    self.calls.push(PipelineCall {
                        name,
                        role,
                        offset: 0, // approximate
                    });
                }
            }
            // Also visit nested calls
            syn::visit::visit_expr_call(self, node);
        }

        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            let name = node.method.to_string();
            let role = classify_call(&name);
            self.calls.push(PipelineCall {
                name,
                role,
                offset: 0,
            });
            syn::visit::visit_expr_method_call(self, node);
        }
    }

    let mut visitor = BodyCallVisitor { calls: Vec::new() };
    for item in items {
        visitor.visit_item(item);
    }
    visitor.calls
}

/// Check pipeline ordering in parsed items.
///
/// Returns warnings for detected ordering issues.
pub fn check_pipeline_ordering(items: &[syn::Item]) -> Vec<PipelineWarning> {
    use kobo_ir::FileId;

    let calls = extract_pipeline_calls(items);
    let mut warnings = Vec::new();
    let dummy_span = KoboSpan::new(0, 0, FileId(0));

    // Track positions: first handler / response position.
    let mut first_handler_idx: Option<usize> = None;
    let mut first_response_idx: Option<usize> = None;

    for (i, call) in calls.iter().enumerate() {
        match call.role {
            CallRole::Handler => {
                if first_handler_idx.is_none() {
                    first_handler_idx = Some(i);
                }
            }
            CallRole::Response => {
                if first_response_idx.is_none() {
                    first_response_idx = Some(i);
                }
            }
            _ => {}
        }
    }

    // Now check for ordering violations.
    for (i, call) in calls.iter().enumerate() {
        match call.role {
            CallRole::Auth => {
                if let Some(handler_idx) = first_handler_idx {
                    if i > handler_idx {
                        warnings.push(PipelineWarning {
                            kind: PipelineIssue::AuthAfterHandler,
                            span: dummy_span,
                            suggestion: format!(
                                "Move `{}` before the handler call for proper auth checking",
                                call.name
                            ),
                        });
                    }
                }
            }
            CallRole::RateLimit => {
                if let Some(handler_idx) = first_handler_idx {
                    if i > handler_idx {
                        warnings.push(PipelineWarning {
                            kind: PipelineIssue::RateLimitAfterHandler,
                            span: dummy_span,
                            suggestion: format!(
                                "Move `{}` before the handler to limit requests early",
                                call.name
                            ),
                        });
                    }
                }
            }
            CallRole::Logging => {
                if let Some(response_idx) = first_response_idx {
                    if i > response_idx {
                        warnings.push(PipelineWarning {
                            kind: PipelineIssue::LogAfterResponse,
                            span: dummy_span,
                            suggestion: format!(
                                "Move `{}` before the response to capture request data",
                                call.name
                            ),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    warnings
}

/// Convenience for tests: parse source code and check pipeline ordering.
pub fn check_pipeline_ordering_from_source(source: &str) -> Vec<PipelineWarning> {
    let file = syn::parse_file(source).expect("Failed to parse source");
    check_pipeline_ordering(&file.items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_auth_after_handler_warns() {
        let code = r#"
async fn serve(req: Request) -> Response {
    let result = handle_request(req).await;
    let authed = authenticate(req).await;
    result
}
"#;
        let warnings = check_pipeline_ordering_from_source(code);
        assert!(!warnings.is_empty());
        assert!(warnings
            .iter()
            .any(|w| w.kind == PipelineIssue::AuthAfterHandler));
    }

    #[test]
    fn pipeline_auth_before_handler_no_warning() {
        let code = r#"
async fn serve(req: Request) -> Response {
    let authed = authenticate(req).await;
    let result = handle_request(req).await;
    result
}
"#;
        let warnings = check_pipeline_ordering_from_source(code);
        assert!(
            warnings.is_empty(),
            "No warnings when auth is before handler"
        );
    }

    #[test]
    fn pipeline_rate_limit_after_handler_warns() {
        let code = r#"
async fn serve(req: Request) -> Response {
    let result = handle_request(req).await;
    rate_limit(req).await;
    result
}
"#;
        let warnings = check_pipeline_ordering_from_source(code);
        assert!(warnings
            .iter()
            .any(|w| w.kind == PipelineIssue::RateLimitAfterHandler));
    }

    #[test]
    fn pipeline_logging_after_response_warns() {
        let code = r#"
async fn serve(req: Request) -> Response {
    let resp = respond_ok(data).await;
    log_request(req).await;
    resp
}
"#;
        let warnings = check_pipeline_ordering_from_source(code);
        assert!(warnings
            .iter()
            .any(|w| w.kind == PipelineIssue::LogAfterResponse));
    }

    #[test]
    fn no_pipeline_warning_on_non_pipeline_code() {
        let code = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let warnings = check_pipeline_ordering_from_source(code);
        assert!(
            warnings.is_empty(),
            "Non-pipeline code should not trigger warnings"
        );
    }

    #[test]
    fn classify_auth_calls() {
        assert_eq!(classify_call("authenticate"), CallRole::Auth);
        assert_eq!(classify_call("verify_token"), CallRole::Auth);
        assert_eq!(classify_call("check_token_validity"), CallRole::Auth);
    }

    #[test]
    fn classify_handler_calls() {
        assert_eq!(classify_call("handle_request"), CallRole::Handler);
        assert_eq!(classify_call("process_data"), CallRole::Handler);
    }
}
