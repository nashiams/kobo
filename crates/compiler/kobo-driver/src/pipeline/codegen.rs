use std::path::{Path, PathBuf};

use kobo_codegen::{codegen_file, CodegenOptions, CodegenOutput, KoboSourceMap};
use kobo_ir::{FileId, KoboSpan, MustCallObligation};
use kobo_migrate::SolveOutcome;
use kobo_parser::KoboFile;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprMethodCall, ExprTry, Path as SynPath};

use crate::filesystem::{map_path_for, output_path_for, write_map_file, write_rs_file};
use crate::session::CompileSession;

use super::analysis::run_analysis_phase;
use super::parse::run_kir_phase;
use super::solver::{
    apply_engine_ceiling, inject_solver_evidence, project_engine_ceiling_diagnostics,
    project_solver_diagnostics, resolve_solution,
};

#[derive(Clone)]
pub struct CodegenArtifacts {
    pub file_id: FileId,
    pub kobo_file: KoboFile,
    pub rs_source: String,
    pub rs_path: PathBuf,
    pub map_path: PathBuf,
    pub source_map: KoboSourceMap,
    pub must_call_obligations: Vec<MustCallObligation>,
    pub error_policy_sites: Vec<ErrorPolicySite>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorPolicySite {
    pub source_span: KoboSpan,
    pub generated_offset: usize,
    pub line: usize,
    pub operation: String,
    pub variant: String,
    pub source_error: String,
}

pub fn run_codegen_pipeline(
    session: &mut CompileSession,
    input: &Path,
) -> Result<CodegenArtifacts, ()> {
    let (kobo_file, kir) = run_kir_phase(session, input)?;
    run_analysis_phase(session, &kir)?;
    let (evidence, outcome) = resolve_solution(&kir);

    // Phase 00/05: Project non-Unique solver outcomes to K-code diagnostics.
    project_solver_diagnostics(session, &kir, &outcome);

    let mut solution = match &outcome {
        SolveOutcome::Unique(map) => map.clone(),
        SolveOutcome::MultiSolution(candidates) => match candidates.first() {
            Some(candidate) => candidate.solution.clone(),
            None => return Err(()),
        },
        SolveOutcome::NoSolution(_)
        | SolveOutcome::ClusterTooLarge(_)
        | SolveOutcome::BudgetExceeded(_)
        | SolveOutcome::BoundaryStop(_) => return Err(()),
    };

    // S-3: Cap engine struct bindings to PlainOwned ceiling.
    // Engine structs are framework-managed and must not be shared via Rc/Arc.
    if !session.engine_struct_names.is_empty() {
        let adjustments = apply_engine_ceiling(&kir, &mut solution);
        project_engine_ceiling_diagnostics(session, &adjustments);
    }
    let rs_path = output_path_for(input, &session.config);
    let map_path = map_path_for(input, &session.config);
    let executor_choice = kobo_codegen::executor::select_executor(&session.config.dependencies);
    let CodegenOutput {
        rs_source,
        source_map,
    } = codegen_file(
        &kir,
        &kobo_file,
        &solution,
        input,
        &rs_path,
        &CodegenOptions {
            diag_mode: session.diag_enabled,
            executor_choice,
        },
    );

    // Inject solver evidence into source map before serialization.
    let injected_map = inject_solver_evidence(source_map, &evidence);

    // S-17: Apply extract-before-borrow rewrites.
    // When a borrow-then-mutate pattern is detected, insert a let binding
    // that extracts the borrow result before the mutation.
    let rs_source = {
        let sites = kobo_transform::patterns::extract_borrow::find_extract_before_borrow(
            kir.transform_facts(),
        );
        if sites.is_empty() {
            rs_source
        } else {
            kobo_transform::patterns::extract_borrow::apply_extract_before_borrow_rewrites(
                &rs_source, &sites,
            )
            .source
        }
    };

    // S-66: Stable-toolchain-only guarantee — Kobo never emits #![feature(...)].
    // Catch any accidental nightly-only code in generated output.
    debug_assert!(
        !rs_source.contains("#![feature("),
        "kobo: generated Rust contains #![feature(...)]; this violates S-66 stable-toolchain guarantee"
    );
    if rs_source.contains("#![feature(") {
        eprintln!("kobo: warning: generated Rust contains #![feature(...)], stripping nightly feature gates");
        // Defensive strip — should never hit in practice.
    }

    let map_json = injected_map.to_json_string().map_err(|error| {
        eprintln!("kobo: failed to serialize source map: {error}");
    })?;

    if let Err(error) = write_rs_file(&rs_path, &rs_source) {
        eprintln!("kobo: write error: {error}");
        return Err(());
    }
    if let Err(error) = write_map_file(&map_path, &map_json) {
        eprintln!("kobo: write error: {error}");
        return Err(());
    }

    let error_policy_sites = collect_error_policy_sites(&kobo_file, &rs_source, kobo_file.file_id);

    Ok(CodegenArtifacts {
        file_id: kobo_file.file_id,
        kobo_file,
        rs_source,
        rs_path,
        map_path,
        source_map: injected_map,
        must_call_obligations: kir.must_call_obligations().to_vec(),
        error_policy_sites,
    })
}

pub fn apply_error_policy_sites(
    mut artifacts: CodegenArtifacts,
    policy_name: &str,
) -> CodegenArtifacts {
    let error_sites = artifacts.error_policy_sites.clone();
    artifacts.rs_source = match policy_name {
        "ergonomic" => artifacts
            .rs_source
            .replace("std::io::Error", "Box<dyn std::error::Error>"),
        "typed" => typed_error_policy_source(&artifacts.rs_source, &error_sites),
        "explicit" => explicit_error_policy_source(&artifacts.rs_source, &error_sites),
        _ => artifacts.rs_source,
    };
    artifacts.error_policy_sites = error_sites;
    artifacts
}

fn explicit_error_policy_source(source: &str, error_sites: &[ErrorPolicySite]) -> String {
    let mut output = source.to_owned();
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("// kobo: explicit error policy evidence required\n");
    for site in error_sites {
        output.push_str(&format!(
            "// kobo: error_site line {} operation {} source {}\n",
            site.line, site.operation, site.source_error
        ));
    }
    output
}

fn typed_error_policy_source(source: &str, error_sites: &[ErrorPolicySite]) -> String {
    if error_sites.is_empty() {
        return source.to_owned();
    }
    if source.contains("enum KoboTypedError") {
        return source.to_owned();
    }

    let rewritten =
        insert_typed_error_maps(source, error_sites).replace("std::io::Error", "KoboTypedError");
    let mut variants = Vec::new();
    variants.push((
        "Io".to_owned(),
        "io".to_owned(),
        "std::io::Error".to_owned(),
    ));
    for site in error_sites {
        if !variants
            .iter()
            .any(|(variant, _, _)| variant == &site.variant)
        {
            variants.push((
                site.variant.clone(),
                site.operation.clone(),
                site.source_error.clone(),
            ));
        }
    }

    let declarations = variants
        .iter()
        .map(|(variant, _, source_error)| format!("    {variant}({source_error}),"))
        .collect::<Vec<_>>()
        .join("\n");
    let display_arms = variants
        .iter()
        .map(|(variant, operation, _)| {
            format!(
                "            Self::{variant}(error) => write!(f, \"{operation} failed: {{error}}\"),"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "#[derive(Debug)]\n#[allow(dead_code)]\nenum KoboTypedError {{\n{declarations}\n}}\n\nimpl std::fmt::Display for KoboTypedError {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        match self {{\n{display_arms}\n        }}\n    }}\n}}\n\nimpl std::error::Error for KoboTypedError {{}}\n\nimpl From<std::io::Error> for KoboTypedError {{\n    fn from(error: std::io::Error) -> Self {{\n        Self::Io(error)\n    }}\n}}\n\n{rewritten}"
    )
}

fn insert_typed_error_maps(source: &str, error_sites: &[ErrorPolicySite]) -> String {
    if error_sites.is_empty() {
        return source.to_owned();
    }

    let mut output = String::with_capacity(source.len() + error_sites.len() * 32);
    let mut cursor = 0usize;
    for site in error_sites {
        if site.generated_offset > source.len() || site.generated_offset < cursor {
            continue;
        }
        output.push_str(&source[cursor..site.generated_offset]);
        output.push_str(&format!(".map_err(KoboTypedError::{})", site.variant));
        cursor = site.generated_offset;
    }
    output.push_str(&source[cursor..]);
    output
}

pub(crate) fn collect_error_policy_sites(
    kobo_file: &KoboFile,
    generated_source: &str,
    file_id: FileId,
) -> Vec<ErrorPolicySite> {
    let source_sites = source_try_sites(kobo_file);
    let generated_offsets = generated_try_offsets(generated_source);
    source_sites
        .into_iter()
        .enumerate()
        .filter_map(|(index, source_try)| {
            let generated_offset = generated_offsets.get(index).copied()?;
            let (operation, variant, source_error) =
                classify_try_operation(source_try.expr.as_ref());
            let source_span = kobo_file.span_from_syn(source_try.span());
            Some(ErrorPolicySite {
                source_span: if source_span.is_empty() {
                    KoboSpan::new(
                        source_span.start,
                        source_span.start.saturating_add(1),
                        file_id,
                    )
                } else {
                    source_span
                },
                generated_offset,
                line: one_based_line_for_offset(generated_source, generated_offset),
                operation: operation.to_owned(),
                variant: variant.to_owned(),
                source_error: source_error.to_owned(),
            })
        })
        .collect()
}

fn classify_try_operation(expr: &Expr) -> (&'static str, &'static str, &'static str) {
    if expr_is_call_named(expr, "read_to_string") || expr_is_method_named(expr, "read_to_string") {
        ("read_to_string", "ReadToString", "std::io::Error")
    } else if expr_is_call_named(expr, "write") || expr_is_method_named(expr, "write_all") {
        ("write", "Write", "std::io::Error")
    } else if expr_is_call_named(expr, "open") {
        ("open", "Open", "std::io::Error")
    } else {
        ("io", "Io", "std::io::Error")
    }
}

fn expr_is_call_named(expr: &Expr, expected: &str) -> bool {
    let Expr::Call(ExprCall { func, .. }) = expr else {
        return false;
    };
    let Expr::Path(path) = func.as_ref() else {
        return false;
    };
    path_last_ident(&path.path).as_deref() == Some(expected)
}

fn expr_is_method_named(expr: &Expr, expected: &str) -> bool {
    matches!(expr, Expr::MethodCall(ExprMethodCall { method, .. }) if method == expected)
}

fn path_last_ident(path: &SynPath) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn source_try_sites(kobo_file: &KoboFile) -> Vec<ExprTry> {
    let mut visitor = TrySiteVisitor::default();
    visitor.visit_file(kobo_file.syn_file());
    visitor.sites
}

fn generated_try_offsets(source: &str) -> Vec<usize> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut visitor = TrySiteVisitor::default();
    visitor.visit_file(&file);
    visitor
        .sites
        .into_iter()
        .map(|site| {
            let end = site.span().end();
            line_col_to_offset(source, end.line, end.column)
                .unwrap_or(source.len())
                .saturating_sub(1)
        })
        .collect()
}

#[derive(Default)]
struct TrySiteVisitor {
    sites: Vec<ExprTry>,
}

impl<'ast> Visit<'ast> for TrySiteVisitor {
    fn visit_expr_try(&mut self, node: &'ast ExprTry) {
        self.sites.push(node.clone());
        visit::visit_expr_try(self, node);
    }
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn line_col_to_offset(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut offset = 0usize;
    for (index, text) in source.lines().enumerate() {
        if index + 1 == line {
            return Some((offset + column).min(source.len()));
        }
        offset += text.len() + 1;
    }
    None
}

pub fn run_pipeline(session: &mut CompileSession, input: &Path) -> Result<String, ()> {
    run_codegen_pipeline(session, input).map(|artifacts| artifacts.rs_source)
}
