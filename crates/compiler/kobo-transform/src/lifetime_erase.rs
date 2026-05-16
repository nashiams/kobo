/// Erase lifetime annotations in script and checked modes.
///
/// Transformation (parameters):
///   fn process(data: &'a str) → fn process(data: String)
///   fn process(items: &[Item]) → fn process(items: Vec<Item>)
///   fn process(config: &Config) → fn process(config: Config)
///
/// Rules:
/// 1. Only applies in Script/Checked mode (not Strict)
/// 2. &str → String
/// 3. &[T] → Vec<T>
/// 4. &T → T (for all other types)
/// 5. &mut T → T (owned, mutable)
/// 6. Lifetime parameters on structs → removed
/// 7. Insert .clone() at non-last usage points (last usage can move)
/// 8. Return types borrowing from erased params → also erased
///
/// Does NOT apply to:
/// - Static references (&'static T → kept as-is)
/// - Raw pointers (*const T, *mut T → kept as-is)
use kobo_ir::{FileId, GuaranteePolicy, KoboSpan};

/// A site where a clone was inserted due to lifetime erasure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloneSite {
    pub binding_name: String,
    pub span: KoboSpan,
    pub original_type: String,
    pub owned_type: String,
}

/// A parameter whose lifetime was erased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErasedParam {
    pub param_name: String,
    pub original_type: String,
    pub owned_type: String,
}

/// Result of lifetime erasure on a function.
#[derive(Clone, Debug)]
pub struct LifetimeErasureResult {
    pub clone_sites: Vec<CloneSite>,
    pub erased_params: Vec<ErasedParam>,
}

impl LifetimeErasureResult {
    pub fn is_empty(&self) -> bool {
        self.clone_sites.is_empty() && self.erased_params.is_empty()
    }

    pub fn merge(&mut self, other: LifetimeErasureResult) {
        self.clone_sites.extend(other.clone_sites);
        self.erased_params.extend(other.erased_params);
    }
}

fn lifetime_erasure_enabled(policy: &GuaranteePolicy) -> bool {
    policy.is_dev() || policy.is_checked()
}

/// Erase a reference type to its owned equivalent.
///
/// Returns (owned_type_string, was_erased).
fn erase_ref_type(ty: &syn::Type) -> Option<(String, String)> {
    if let syn::Type::Reference(ref_ty) = ty {
        // Skip &'static T
        if let Some(lt) = &ref_ty.lifetime {
            if lt.ident == "static" {
                return None;
            }
        }

        let inner = &ref_ty.elem;
        let original = type_to_string(ty);

        // &str → String
        if is_str_type(inner) {
            return Some((original, "String".to_owned()));
        }

        // &[T] → Vec<T>
        if let syn::Type::Slice(slice) = inner.as_ref() {
            let elem = type_to_string(&slice.elem);
            return Some((original, format!("Vec<{elem}>")));
        }

        // &T → T (general case)
        let inner_str = type_to_string(inner);
        return Some((original, inner_str));
    }
    None
}

/// Check if a type is `str` (the unsized string slice type).
fn is_str_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(path) = ty {
        if let Some(seg) = path.path.segments.last() {
            return seg.ident == "str";
        }
    }
    false
}

/// Convert a syn::Type to a string representation.
fn type_to_string(ty: &syn::Type) -> String {
    use quote::ToTokens;
    ty.to_token_stream().to_string()
}

/// Erase lifetimes in a function's parameter list and return type.
///
/// Returns the list of erased params (including return type if applicable).
fn erase_fn_params(sig: &syn::Signature) -> Vec<ErasedParam> {
    let mut erased = Vec::new();
    for arg in &sig.inputs {
        if let syn::FnArg::Typed(pat_type) = arg {
            if let Some((original, owned)) = erase_ref_type(&pat_type.ty) {
                let param_name = pat_to_string(&pat_type.pat);
                erased.push(ErasedParam {
                    param_name,
                    original_type: original,
                    owned_type: owned,
                });
            }
        }
    }
    // Rule 8: Erase return type references too.
    if let syn::ReturnType::Type(_, ref ret_ty) = sig.output {
        if let Some((original, owned)) = erase_ref_type(ret_ty) {
            erased.push(ErasedParam {
                param_name: "<return>".to_owned(),
                original_type: original,
                owned_type: owned,
            });
        }
    }
    erased
}

/// Convert a pattern to its string representation.
fn pat_to_string(pat: &syn::Pat) -> String {
    use quote::ToTokens;
    pat.to_token_stream().to_string()
}

/// Count usages of a binding name in a block.
fn count_usages_in_block(block: &syn::Block, name: &str) -> usize {
    use syn::visit::Visit;

    struct UsageCounter<'a> {
        name: &'a str,
        count: usize,
    }

    impl<'ast, 'a> syn::visit::Visit<'ast> for UsageCounter<'a> {
        fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
            if ident == self.name {
                self.count += 1;
            }
        }
    }

    let mut counter = UsageCounter { name, count: 0 };
    counter.visit_block(block);
    counter.count
}

/// Erase lifetimes on a syn::Item.
///
/// Only applies in Script/Checked mode. Returns None if:
/// - Mode is Strict
/// - The item is not a function
/// - No references to erase
pub fn erase_lifetimes(
    item: &syn::Item,
    policy: &GuaranteePolicy,
) -> Option<LifetimeErasureResult> {
    if !lifetime_erasure_enabled(policy) {
        return None;
    }

    let func = match item {
        syn::Item::Fn(f) => f,
        _ => return None,
    };

    let erased_params = erase_fn_params(&func.sig);
    if erased_params.is_empty() {
        return None;
    }

    // Count clone sites: for each erased param, count usages in the body.
    // Last usage can move; all others need .clone().
    let mut clone_sites = Vec::new();
    let dummy_span = KoboSpan::new(0, 0, FileId(0));

    for param in &erased_params {
        let usage_count = count_usages_in_block(&func.block, &param.param_name);
        // Subtract 1 for the last usage (which can move).
        let clone_count = usage_count.saturating_sub(1);
        for _ in 0..clone_count {
            clone_sites.push(CloneSite {
                binding_name: param.param_name.clone(),
                span: dummy_span,
                original_type: param.original_type.clone(),
                owned_type: param.owned_type.clone(),
            });
        }
    }

    Some(LifetimeErasureResult {
        clone_sites,
        erased_params,
    })
}

/// Analyze all functions in a source file for lifetime erasure clone/debt data.
pub fn analyze_source(source: &str, policy: &GuaranteePolicy) -> LifetimeErasureResult {
    let mut combined = LifetimeErasureResult {
        clone_sites: Vec::new(),
        erased_params: Vec::new(),
    };

    if !lifetime_erasure_enabled(policy) {
        return combined;
    }

    let Ok(file) = syn::parse_file(source) else {
        return combined;
    };

    for item in &file.items {
        if let Some(result) = erase_lifetimes(item, policy) {
            combined.merge(result);
        }
    }

    combined
}

/// Rewrite a function signature string with erased lifetimes.
///
/// This is the source-level transformation: replaces &str with String, etc.
/// Uses syn parsing to identify reference types, then does source-level text replacement.
/// Rule 8: Return types borrowing from erased params are also erased.
pub fn rewrite_fn_signature(source: &str, policy: &GuaranteePolicy) -> String {
    if !lifetime_erasure_enabled(policy) {
        return source.to_owned();
    }

    let file = match syn::parse_file(source) {
        Ok(f) => f,
        Err(_) => return source.to_owned(),
    };

    let mut result = source.to_owned();

    for item in &file.items {
        if let syn::Item::Fn(func) = item {
            // Erase parameter types
            for arg in &func.sig.inputs {
                if let syn::FnArg::Typed(pat_type) = arg {
                    if let Some(replacement) = source_level_erase(&pat_type.ty, &result) {
                        result = replacement;
                    }
                }
            }
            // Rule 8: Also erase return type references
            if let syn::ReturnType::Type(_, ref ret_ty) = func.sig.output {
                if let Some(replacement) = source_level_erase(ret_ty, &result) {
                    result = replacement;
                }
            }
        }
    }

    result
}

/// Perform source-level replacement of a reference type with its owned equivalent.
///
/// We search for known textual patterns of the reference in the source.
fn source_level_erase(ty: &syn::Type, source: &str) -> Option<String> {
    if let syn::Type::Reference(ref_ty) = ty {
        // Skip &'static T
        if let Some(lt) = &ref_ty.lifetime {
            if lt.ident == "static" {
                return None;
            }
        }

        let inner = &ref_ty.elem;

        // Determine the owned type replacement.
        let owned = if is_str_type(inner) {
            "String".to_owned()
        } else if let syn::Type::Slice(slice) = inner.as_ref() {
            let elem = type_to_string(&slice.elem).replace(' ', "");
            format!("Vec<{elem}>")
        } else {
            type_to_string(inner).replace(' ', "")
        };

        // Build candidate patterns to search for in source.
        // We try multiple common textual representations.
        let candidates = build_search_candidates(ref_ty);

        for candidate in &candidates {
            if let Some(pos) = source.find(candidate.as_str()) {
                let mut result = String::with_capacity(source.len());
                result.push_str(&source[..pos]);
                result.push_str(&owned);
                result.push_str(&source[pos + candidate.len()..]);
                return Some(result);
            }
        }
    }
    None
}

/// Build candidate search strings for a reference type.
fn build_search_candidates(ref_ty: &syn::TypeReference) -> Vec<String> {
    let inner = &ref_ty.elem;
    let mut candidates = Vec::new();

    let inner_str = if is_str_type(inner) {
        "str".to_owned()
    } else if let syn::Type::Slice(slice) = inner.as_ref() {
        let elem = type_to_string(&slice.elem).replace(' ', "");
        format!("[{elem}]")
    } else {
        type_to_string(inner).replace(' ', "")
    };

    if let Some(lt) = &ref_ty.lifetime {
        let lt_str = lt.ident.to_string();
        // &'a str, &'a T, etc.
        candidates.push(format!("&'{lt_str} {inner_str}"));
        candidates.push(format!("&'{lt_str}  {inner_str}"));
    }

    if ref_ty.mutability.is_some() {
        candidates.push(format!("&mut {inner_str}"));
        candidates.push(format!("& mut {inner_str}"));
    }

    // Basic: &str, &T, &[T]
    candidates.push(format!("&{inner_str}"));
    candidates.push(format!("& {inner_str}"));

    candidates
}

/// Format a clone debt report line for a binding.
pub fn format_clone_debt(clone_sites: &[CloneSite]) -> String {
    use std::collections::HashMap;

    if clone_sites.is_empty() {
        return String::new();
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for site in clone_sites {
        *counts.entry(&site.binding_name).or_insert(0) += 1;
    }

    let mut lines = Vec::new();
    lines.push("Lifetime Erasure Clones:".to_owned());

    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1)); // sort by count descending

    for (name, count) in sorted {
        let advice = if count > 10 {
            "Consider storing a reference"
        } else {
            "Acceptable for prototyping"
        };
        lines.push(format!("  {name} ({count} clones) — {advice}"));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::GuaranteeProfile;

    fn policy(profile: GuaranteeProfile) -> GuaranteePolicy {
        GuaranteePolicy::for_profile(profile)
    }

    #[test]
    fn ref_str_to_string() {
        let input = r#"
fn greet(name: &str) {
    println!("Hello, {}", name);
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(output.contains("fn greet(name: String)"), "got: {output}");
    }

    #[test]
    fn slice_to_vec() {
        let input = r#"
fn sum(items: &[i32]) -> i32 {
    items.iter().sum()
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("fn sum(items: Vec<i32>)")
                || output.contains("fn sum(items : Vec < i32 >)"),
            "got: {output}"
        );
    }

    #[test]
    fn erasure_applies_in_checked_mode() {
        let input = r#"
fn greet(name: &str) {
    println!("Hello, {}", name);
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Checked));
        assert!(
            output.contains("String") && !output.contains("&str"),
            "Checked mode should erase public &str, got: {output}"
        );
    }

    #[test]
    fn no_erasure_in_strict_mode() {
        let input = r#"
fn greet(name: &str) {
    println!("Hello, {}", name);
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Release));
        assert!(
            output.contains("&str"),
            "Strict mode should keep &str, got: {output}"
        );
    }

    #[test]
    fn clone_count_in_debt() {
        let input = r#"
fn process(data: &str) {
    let a = data;
    let b = data;
    let c = data;
}
"#;
        let file = syn::parse_file(input).unwrap();
        let result = erase_lifetimes(&file.items[0], &policy(GuaranteeProfile::Dev));
        let result = result.expect("Should have erasure result");
        // data is used 3 times in the body → 2 clones (last can move)
        assert_eq!(
            result.clone_sites.len(),
            2,
            "Expected 2 clone sites for 3 usages"
        );

        let report = format_clone_debt(&result.clone_sites);
        assert!(
            report.contains("data") && report.contains("2 clones"),
            "got: {report}"
        );
    }

    #[test]
    fn static_ref_not_erased() {
        let input = r#"
fn label(s: &'static str) {
    println!("{}", s);
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("&'static str") || output.contains("& 'static str"),
            "Static refs should be preserved, got: {output}"
        );
    }

    #[test]
    fn ref_t_to_owned() {
        let input = r#"
fn take(config: &Config) {
    use_config(config);
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("config: Config") || output.contains("config : Config"),
            "got: {output}"
        );
    }

    #[test]
    fn format_clone_debt_empty() {
        let report = format_clone_debt(&[]);
        assert!(report.is_empty());
    }

    #[test]
    fn format_clone_debt_high_count() {
        let sites: Vec<CloneSite> = (0..15)
            .map(|_| CloneSite {
                binding_name: "source".to_owned(),
                span: KoboSpan::new(0, 0, FileId(0)),
                original_type: "&str".to_owned(),
                owned_type: "String".to_owned(),
            })
            .collect();
        let report = format_clone_debt(&sites);
        assert!(report.contains("source"));
        assert!(report.contains("15 clones"));
        assert!(report.contains("Consider storing a reference"));
    }

    // ─── BUG-02 tests: return type erasure ───

    #[test]
    fn return_ref_str_erased_to_string() {
        let input = r#"
fn name(data: &str) -> &str {
    data
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("-> String"),
            "Return type &str should become String, got: {output}"
        );
        assert!(
            !output.contains("-> &str"),
            "Return &str should be erased, got: {output}"
        );
    }

    #[test]
    fn return_ref_slice_erased_to_vec() {
        let input = r#"
fn items(data: &[i32]) -> &[i32] {
    data
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("-> Vec<i32>") || output.contains("-> Vec < i32 >"),
            "Return type &[i32] should become Vec<i32>, got: {output}"
        );
    }

    #[test]
    fn return_ref_t_erased_to_owned() {
        let input = r#"
fn get_config(cfg: &Config) -> &Config {
    cfg
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("-> Config"),
            "Return type &Config should become Config, got: {output}"
        );
    }

    #[test]
    fn return_static_ref_not_erased() {
        let input = r#"
fn label() -> &'static str {
    "hello"
}
"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("&'static str") || output.contains("& 'static str"),
            "Return &'static str should be preserved, got: {output}"
        );
    }

    #[test]
    fn erase_lifetimes_includes_return_type() {
        let input = r#"
fn first(items: &[u8]) -> &u8 {
    &items[0]
}
"#;
        let file = syn::parse_file(input).unwrap();
        let result = erase_lifetimes(&file.items[0], &policy(GuaranteeProfile::Dev));
        let result = result.expect("Should have erasure result");
        // The erased_params should include the return type erasure
        assert!(
            result
                .erased_params
                .iter()
                .any(|p| p.original_type.contains("&") && p.owned_type.contains("u8")),
            "erased_params: {:?}",
            result.erased_params
        );
    }

    // ─── v0.8 edge-case tests ───

    /// &'static str is NEVER erased (Trap 5).
    #[test]
    fn static_lifetime_always_preserved() {
        let input = r#"fn get_name() -> &'static str { "hello" }"#;
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("&'static str"),
            "&'static must be preserved, got: {output}"
        );
    }

    /// &str → String in Script mode.
    #[test]
    fn ref_str_becomes_string_script() {
        let input = "fn greet(name: &str) { }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(output.contains("String"), "got: {output}");
        assert!(!output.contains("&str"), "got: {output}");
    }

    /// &[T] → Vec<T> in Script mode.
    #[test]
    fn ref_slice_becomes_vec_script() {
        let input = "fn sum(items: &[f64]) -> f64 { 0.0 }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("Vec<f64>") || output.contains("Vec < f64 >"),
            "got: {output}"
        );
    }

    /// &T → T for arbitrary struct types.
    #[test]
    fn ref_struct_becomes_owned() {
        let input = "fn process(config: &Config) { }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            output.contains("config: Config") || output.contains("config : Config"),
            "got: {output}"
        );
        assert!(!output.contains("&Config"), "got: {output}");
    }

    /// Return type references erased too (Rule 8).
    #[test]
    fn return_type_ref_erased() {
        let input = "fn get_data(data: &str) -> &str { data }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        let ref_count = output.matches("&str").count();
        assert_eq!(ref_count, 0, "all &str should be erased, got: {output}");
    }

    /// Multiple reference params all erased.
    #[test]
    fn multiple_ref_params_all_erased() {
        let input = "fn combine(a: &str, b: &str) -> String { format!(\"{}{}\", a, b) }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        let ref_count = output.matches("&str").count();
        assert_eq!(ref_count, 0, "all &str should be erased, got: {output}");
    }

    /// &mut T → T in Script mode.
    #[test]
    fn mut_ref_erased() {
        let input = "fn update(data: &mut Vec<i32>) { }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            !output.contains("&mut"),
            "&mut should be erased in Script, got: {output}"
        );
    }

    /// Non-fn item → erase_lifetimes returns None.
    #[test]
    fn non_fn_item_returns_none() {
        let code = "struct Foo { x: i32 }";
        let file = syn::parse_file(code).unwrap();
        for item in &file.items {
            assert!(erase_lifetimes(item, &policy(GuaranteeProfile::Dev)).is_none());
        }
    }

    /// Function with no references → None.
    #[test]
    fn no_refs_returns_none() {
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let file = syn::parse_file(code).unwrap();
        for item in &file.items {
            assert!(erase_lifetimes(item, &policy(GuaranteeProfile::Dev)).is_none());
        }
    }

    /// Clone debt: 3 usages → 2 clones.
    #[test]
    fn clone_debt_three_usages_two_clones() {
        let input = r#"
fn process(data: &str) {
    let a = data;
    let b = data;
    let c = data;
}
"#;
        let file = syn::parse_file(input).unwrap();
        let result = erase_lifetimes(&file.items[0], &policy(GuaranteeProfile::Dev));
        let result = result.expect("Should have erasure result");
        assert_eq!(
            result.clone_sites.len(),
            2,
            "Expected 2 clone sites for 3 usages"
        );
    }

    /// format_clone_debt output format.
    #[test]
    fn clone_debt_format() {
        let sites = vec![
            CloneSite {
                binding_name: "data".to_owned(),
                span: KoboSpan::new(0, 0, FileId(0)),
                original_type: "&str".to_owned(),
                owned_type: "String".to_owned(),
            },
            CloneSite {
                binding_name: "data".to_owned(),
                span: KoboSpan::new(0, 0, FileId(0)),
                original_type: "&str".to_owned(),
                owned_type: "String".to_owned(),
            },
        ];
        let report = format_clone_debt(&sites);
        assert!(report.contains("data"), "report: {report}");
        assert!(report.contains("2 clones"), "report: {report}");
    }

    /// format_clone_debt with empty sites.
    #[test]
    fn clone_debt_empty() {
        let report = format_clone_debt(&[]);
        assert!(report.is_empty());
    }

    /// &[u8] → Vec<u8>.
    #[test]
    fn ref_u8_slice_becomes_vec_u8() {
        let input = "fn process(data: &[u8]) { }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(output.contains("Vec<u8>"), "got: {output}");
    }

    /// Named lifetime &'a T → T (not static, so erased).
    #[test]
    fn named_lifetime_erased() {
        let input = "fn process<'a>(data: &'a Config) -> &'a Config { data }";
        let output = rewrite_fn_signature(input, &policy(GuaranteeProfile::Dev));
        assert!(
            !output.contains("&'a"),
            "&'a should be erased in Script, got: {output}"
        );
    }
}
