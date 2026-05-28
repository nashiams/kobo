use std::collections::{HashMap, HashSet};

use kobo_ir::{
    KoboSpan, MustCallObligation, ProtocolTemplateRegistry, ScenarioBoundary,
    ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioCallGraphScc,
    ScenarioCoreTerminatorKind, ScenarioCoverageFacts, ScenarioExternalCallShape,
    ScenarioLifecycleTemplate, ScenarioModeledBoundary, ScenarioOp, ScenarioOpKind,
    ScenarioProgram,
};
use kobo_parser::KoboFile;
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self};
use syn::{
    Block, Expr, ExprAsync, ExprCall, ExprIf, ExprLit, ExprMatch, ExprMethodCall, ExprPath,
    ExprStruct, ExprTry, File, Item, ItemFn, ItemUse, Lit, Local, Macro, Meta, MetaNameValue, Pat,
    PatIdent, PatType, Path, Stmt, UseTree,
};

mod boundary_policy;
mod call_graph;
mod calls;
mod collect;
mod control_flow;
mod env;
mod expr_lower;
mod external_boundary;
mod imports;
mod lifecycle;
mod lowerer;
mod structural_expr;
mod syntax;
#[cfg(test)]
mod tests;

pub use lowerer::build_scenario_programs;

use boundary_policy::collect_boundary_policies;
use collect::{collect_functions, collect_method_shapes, must_call_type_map};
use control_flow::loop_label;
use imports::{collect_use_crate_aliases, collect_use_tree_aliases};
use lifecycle::{
    drop_discharge_action, handler_reply_actions, is_tokio_spawn, peel_paren_expr,
    terminal_action_name,
};
use syntax::{
    boundaries_from_operations, expr_path_ident, fn_arg_ident, function_returns_bool_literal,
    is_handler_obligation_argument, local_suppression_reason, matches_bool_pat, modeled_boundary,
    must_call_actions, pat_ident, path_ends_with, path_ends_with_segments, path_first_ident,
    path_last_ident, path_starts_with, path_to_string, receiver_has_ward_member, receiver_ident,
    select_branch_count,
};

type BindingMap = HashMap<String, String>;
type BoolMap = HashMap<String, bool>;
type ActionMap = HashMap<String, Vec<String>>;
type ExternalBindingMap = HashMap<String, ExternalBoundaryValue>;
type ImportMap = HashMap<String, Vec<String>>;
type BoundaryPolicyMap = HashMap<String, BoundaryPolicyFact>;
type FunctionMap<'a> = HashMap<String, &'a ItemFn>;
type FunctionSccMap = HashMap<String, usize>;
type MethodShapeMap = HashMap<String, HashMap<String, MethodShape>>;

#[derive(Clone, Default)]
struct BindingEnv {
    bindings: BindingMap,
    bools: BoolMap,
    terminal_actions: ActionMap,
    external_values: ExternalBindingMap,
    imports: ImportMap,
    local_types: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct ExternalBoundaryValue {
    crate_name: String,
    type_path: String,
}

#[derive(Clone, Debug)]
struct BoundaryPolicyFact {
    policy: ScenarioBoundaryPolicy,
    reason: Option<String>,
}

#[derive(Clone, Debug)]
struct FunctionScc {
    functions: Vec<String>,
    is_recursive: bool,
}

#[derive(Clone, Debug)]
struct ScenarioCallGraph {
    sccs: Vec<FunctionScc>,
    function_sccs: FunctionSccMap,
}

struct InferredLifecycleCreation {
    binding: String,
    type_name: String,
    actions: Vec<String>,
    template: ScenarioLifecycleTemplate,
    span: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BlockFlow {
    Fallthrough,
    Continue { loop_id: String },
    Break { loop_id: String },
    Return,
    Mixed { flows: Vec<BlockFlow> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LoopFrame {
    id: String,
    label: Option<String>,
}

#[derive(Clone, Debug)]
struct MethodShape {
    return_type: Option<String>,
    consumes_receiver: bool,
}

#[derive(Clone, Debug)]
struct UnsupportedContainerShape {
    type_name: String,
    container: String,
}

#[derive(Default)]
struct TarjanState {
    next_index: usize,
    stack: Vec<String>,
    indices: HashMap<String, usize>,
    lowlinks: HashMap<String, usize>,
    on_stack: HashSet<String>,
    components: Vec<Vec<String>>,
}

struct DirectCallVisitor<'a> {
    known_functions: &'a HashSet<String>,
    calls: Vec<String>,
}

struct ScenarioLowerer<'a> {
    ast: &'a KoboFile,
    must_call_types: &'a HashMap<String, Vec<String>>,
    functions: &'a FunctionMap<'a>,
    call_graph: &'a ScenarioCallGraph,
    imports: &'a ImportMap,
    boundary_policies: &'a BoundaryPolicyMap,
    method_shapes: &'a MethodShapeMap,
    operations: Vec<ScenarioOp>,
    coverage: ScenarioCoverageFacts,
    active_functions: Vec<String>,
    loop_stack: Vec<LoopFrame>,
    next_loop_id: usize,
}
