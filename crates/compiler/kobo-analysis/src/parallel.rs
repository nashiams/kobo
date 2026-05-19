use std::collections::HashMap;

use quote::ToTokens;
use syn::visit::Visit;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParallelWarning {
    pub kind: ParallelWarningKind,
    pub source_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParallelWarningKind {
    NonSendCapture {
        binding_name: String,
        type_name: String,
    },
    SharedMutation {
        binding_name: String,
    },
    MissingBoundaryPolicy,
}

#[derive(Clone, Debug)]
struct BindingFact {
    name: String,
    type_name: Option<String>,
    is_mutable: bool,
}

#[derive(Clone, Debug)]
struct LineInfo<'a> {
    text: &'a str,
    offset: usize,
}

pub fn scan_source_parallel_warnings(source: &str) -> Vec<ParallelWarning> {
    if let Ok(file) = syn::parse_file(source) {
        let mut scanner = ParallelAstScanner {
            source,
            bindings: Vec::new(),
            non_send_types: HashMap::new(),
            warnings: Vec::new(),
        };
        scanner.visit_file(&file);
        return scanner.warnings;
    }

    let lines = line_infos(source);
    let mut warnings = Vec::new();
    let mut bindings = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.text.trim();
        if let Some(binding) = parse_binding(trimmed) {
            bindings.push(binding);
        }
        if !trimmed.contains("#[kobo::parallel") {
            continue;
        }
        if parallel_attr_has_key(trimmed, "order") {
            continue;
        }
        let has_policy = parallel_attr_has_key(trimmed, "policy");
        let Some(loop_extent) = loop_extent(&lines, index + 1) else {
            continue;
        };
        warnings.extend(loop_warnings(source, &bindings, has_policy, loop_extent));
    }

    warnings
}

struct ParallelAstScanner<'a> {
    source: &'a str,
    bindings: Vec<BindingFact>,
    non_send_types: HashMap<String, String>,
    warnings: Vec<ParallelWarning>,
}

impl<'ast> Visit<'ast> for ParallelAstScanner<'_> {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let old_len = self.bindings.len();
        self.bindings
            .extend(item.sig.inputs.iter().filter_map(binding_from_fn_arg));
        syn::visit::visit_block(self, &item.block);
        self.bindings.truncate(old_len);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let old_len = self.bindings.len();
        self.bindings
            .extend(item.sig.inputs.iter().filter_map(binding_from_fn_arg));
        syn::visit::visit_block(self, &item.block);
        self.bindings.truncate(old_len);
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        if struct_has_non_send_field(item) {
            self.non_send_types
                .insert(item.ident.to_string(), item.ident.to_string());
        }
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(binding) = binding_from_local(local, &self.non_send_types) {
            self.bindings.push(binding);
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        let Some(attr) = node.attrs.iter().find(|attr| is_parallel_attr(attr)) else {
            syn::visit::visit_expr_for_loop(self, node);
            return;
        };
        if attr_has_key(attr, "order") {
            syn::visit::visit_expr_for_loop(self, node);
            return;
        }

        let has_policy = attr_has_key(attr, "policy");
        let body_source = node.body.to_token_stream().to_string();
        let iterator_source = iterator_source_ident(node.expr.as_ref());
        if !has_policy && body_source.contains("ward") {
            self.warnings.push(ParallelWarning {
                kind: ParallelWarningKind::MissingBoundaryPolicy,
                source_offset: token_offset(self.source, "ward").unwrap_or(0),
            });
        }

        for binding in &self.bindings {
            if binding.type_name.is_some()
                && (contains_ident(&body_source, &binding.name)
                    || iterator_source.as_deref() == Some(binding.name.as_str()))
            {
                self.warnings.push(ParallelWarning {
                    kind: ParallelWarningKind::NonSendCapture {
                        binding_name: binding.name.clone(),
                        type_name: binding.type_name.clone().unwrap_or_default(),
                    },
                    source_offset: token_offset(self.source, &binding.name).unwrap_or(0),
                });
            }
            if binding.is_mutable && block_mutates_binding(&node.body, &binding.name) {
                self.warnings.push(ParallelWarning {
                    kind: ParallelWarningKind::SharedMutation {
                        binding_name: binding.name.clone(),
                    },
                    source_offset: mutation_offset(self.source, &binding.name)
                        .or_else(|| token_offset(self.source, &binding.name))
                        .unwrap_or(0),
                });
            }
        }

        syn::visit::visit_expr_for_loop(self, node);
    }
}

struct BodyMutationVisitor<'a> {
    binding: &'a str,
    found: bool,
}

impl<'ast> Visit<'ast> for BodyMutationVisitor<'_> {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if receiver_matches_binding(node.receiver.as_ref(), self.binding)
            && mutating_method(&node.method)
        {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        if receiver_matches_binding(node.left.as_ref(), self.binding) {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_assign(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        if is_compound_assignment(&node.op)
            && receiver_matches_binding(node.left.as_ref(), self.binding)
        {
            self.found = true;
            return;
        }
        syn::visit::visit_expr_binary(self, node);
    }
}

fn block_mutates_binding(block: &syn::Block, binding: &str) -> bool {
    let mut visitor = BodyMutationVisitor {
        binding,
        found: false,
    };
    visitor.visit_block(block);
    visitor.found
}

fn receiver_matches_binding(expr: &syn::Expr, binding: &str) -> bool {
    match expr {
        syn::Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => path
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == binding),
        syn::Expr::Field(field) => receiver_matches_binding(field.base.as_ref(), binding),
        syn::Expr::Index(index) => receiver_matches_binding(index.expr.as_ref(), binding),
        syn::Expr::Paren(paren) => receiver_matches_binding(paren.expr.as_ref(), binding),
        syn::Expr::Group(group) => receiver_matches_binding(group.expr.as_ref(), binding),
        _ => false,
    }
}

fn mutating_method(method: &syn::Ident) -> bool {
    matches!(
        method.to_string().as_str(),
        "push" | "insert" | "extend" | "remove" | "pop" | "clear" | "retain" | "truncate"
    )
}

fn is_compound_assignment(op: &syn::BinOp) -> bool {
    matches!(
        op,
        syn::BinOp::AddAssign(_)
            | syn::BinOp::SubAssign(_)
            | syn::BinOp::MulAssign(_)
            | syn::BinOp::DivAssign(_)
            | syn::BinOp::RemAssign(_)
            | syn::BinOp::BitXorAssign(_)
            | syn::BinOp::BitAndAssign(_)
            | syn::BinOp::BitOrAssign(_)
            | syn::BinOp::ShlAssign(_)
            | syn::BinOp::ShrAssign(_)
    )
}

fn is_parallel_attr(attr: &syn::Attribute) -> bool {
    let segments = attr.path().segments.iter().collect::<Vec<_>>();
    segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "parallel"
}

fn attr_has_key(attr: &syn::Attribute, key: &str) -> bool {
    attr_value(attr, key).is_some()
}

fn attr_value(attr: &syn::Attribute, key: &str) -> Option<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    for entry in entries {
        if entry.path.get_ident().is_none_or(|ident| ident != key) {
            continue;
        }
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        return Some(value.value());
    }
    None
}

fn struct_has_non_send_field(item: &syn::ItemStruct) -> bool {
    item.fields.iter().any(|field| {
        let tokens = field.ty.to_token_stream().to_string();
        is_non_send_type_tokens(&tokens)
    })
}

fn binding_from_local(
    local: &syn::Local,
    non_send_types: &HashMap<String, String>,
) -> Option<BindingFact> {
    let (name, is_mutable, type_hint) = binding_name_from_pat(&local.pat)?;
    let init_tokens = local
        .init
        .as_ref()
        .map(|init| init.expr.to_token_stream().to_string())
        .unwrap_or_default();
    let type_tokens = type_hint.unwrap_or_default();
    let type_name = if is_non_send_type_tokens(&type_tokens)
        || init_tokens.contains("Rc :: new")
        || init_tokens.contains("std :: rc :: Rc")
    {
        Some("Rc".to_owned())
    } else if let Some(type_name) =
        constructed_non_send_type(&type_tokens, &init_tokens, non_send_types)
    {
        Some(type_name)
    } else {
        None
    };
    Some(BindingFact {
        name,
        type_name,
        is_mutable,
    })
}

fn binding_from_fn_arg(arg: &syn::FnArg) -> Option<BindingFact> {
    let syn::FnArg::Typed(argument) = arg else {
        return None;
    };
    let syn::Pat::Ident(ident) = argument.pat.as_ref() else {
        return None;
    };
    let type_tokens = argument.ty.to_token_stream().to_string();
    let type_name = is_non_send_type_tokens(&type_tokens).then(|| {
        if type_tokens.contains("Rc") {
            "Rc".to_owned()
        } else if type_tokens.contains("RefCell") {
            "RefCell".to_owned()
        } else {
            "Cell".to_owned()
        }
    });
    Some(BindingFact {
        name: ident.ident.to_string(),
        type_name,
        is_mutable: ident.mutability.is_some(),
    })
}

fn binding_name_from_pat(pat: &syn::Pat) -> Option<(String, bool, Option<String>)> {
    match pat {
        syn::Pat::Ident(ident) => Some((ident.ident.to_string(), ident.mutability.is_some(), None)),
        syn::Pat::Type(pat_type) => {
            let syn::Pat::Ident(ident) = pat_type.pat.as_ref() else {
                return None;
            };
            Some((
                ident.ident.to_string(),
                ident.mutability.is_some(),
                Some(pat_type.ty.to_token_stream().to_string()),
            ))
        }
        _ => None,
    }
}

fn is_non_send_type_tokens(tokens: &str) -> bool {
    tokens.contains("Rc")
        || tokens.contains("std :: rc :: Rc")
        || tokens.contains("RefCell")
        || tokens.contains("Cell")
}

fn constructed_non_send_type(
    type_tokens: &str,
    init_tokens: &str,
    non_send_types: &HashMap<String, String>,
) -> Option<String> {
    for type_name in non_send_types.keys() {
        if contains_type_token(type_tokens, type_name)
            || init_tokens.starts_with(&format!("{type_name} ::"))
            || init_tokens.starts_with(&format!("{type_name} {{"))
        {
            return Some(type_name.clone());
        }
    }
    None
}

fn contains_type_token(tokens: &str, type_name: &str) -> bool {
    tokens
        .split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
        .any(|part| part == type_name)
}

fn token_offset(source: &str, token: &str) -> Option<usize> {
    ident_offset(source, token)
}

fn iterator_source_ident(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::MethodCall(method_call) => {
            if method_call.method == "iter" && method_call.args.is_empty() {
                let syn::Expr::Path(path) = method_call.receiver.as_ref() else {
                    return None;
                };
                if path.qself.is_none() && path.path.segments.len() == 1 {
                    return path
                        .path
                        .segments
                        .first()
                        .map(|segment| segment.ident.to_string());
                }
            }
            iterator_source_ident(method_call.receiver.as_ref())
        }
        syn::Expr::Paren(paren) => iterator_source_ident(paren.expr.as_ref()),
        syn::Expr::Group(group) => iterator_source_ident(group.expr.as_ref()),
        _ => None,
    }
}

fn loop_warnings(
    source: &str,
    bindings: &[BindingFact],
    has_policy: bool,
    extent: (usize, usize),
) -> Vec<ParallelWarning> {
    let loop_source = &source[extent.0..extent.1.min(source.len())];
    let mut warnings = Vec::new();
    if !has_policy && loop_source.contains("ward.") {
        let ward_offset = extent.0 + loop_source.find("ward.").unwrap_or(0);
        warnings.push(ParallelWarning {
            kind: ParallelWarningKind::MissingBoundaryPolicy,
            source_offset: ward_offset,
        });
    }

    for binding in bindings {
        if binding
            .type_name
            .as_deref()
            .is_some_and(|type_name| type_name == "Rc")
            && contains_ident(loop_source, &binding.name)
        {
            warnings.push(ParallelWarning {
                kind: ParallelWarningKind::NonSendCapture {
                    binding_name: binding.name.clone(),
                    type_name: "Rc".to_owned(),
                },
                source_offset: extent.0 + ident_offset(loop_source, &binding.name).unwrap_or(0),
            });
        }
        if binding.is_mutable {
            if let Some(offset) = mutation_offset(loop_source, &binding.name) {
                warnings.push(ParallelWarning {
                    kind: ParallelWarningKind::SharedMutation {
                        binding_name: binding.name.clone(),
                    },
                    source_offset: extent.0 + offset,
                });
            }
        }
    }
    warnings
}

fn line_infos(source: &str) -> Vec<LineInfo<'_>> {
    let mut offset = 0usize;
    source
        .split_inclusive('\n')
        .map(|line| {
            let info = LineInfo { text: line, offset };
            offset += line.len();
            info
        })
        .collect()
}

fn parse_binding(line: &str) -> Option<BindingFact> {
    let rest = line.strip_prefix("let ")?;
    let (is_mutable, rest) = match rest.strip_prefix("mut ") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let name = rest
        .split([':', '=', ' '])
        .next()
        .filter(|name| !name.is_empty())?
        .to_owned();
    let type_name = if rest.contains("Rc::new") {
        Some("Rc".to_owned())
    } else if rest.contains("Arc::new") {
        Some("Arc".to_owned())
    } else {
        None
    };
    Some(BindingFact {
        name,
        type_name,
        is_mutable,
    })
}

fn parallel_attr_has_key(line: &str, key: &str) -> bool {
    line.contains(&format!("{key} =")) || line.contains(&format!("{key}="))
}

fn loop_extent(lines: &[LineInfo<'_>], start_index: usize) -> Option<(usize, usize)> {
    let mut start = None;
    let mut depth = 0isize;
    for line in &lines[start_index..] {
        if start.is_none() && line.text.contains("for ") {
            start = Some(line.offset);
        }
        if start.is_none() {
            continue;
        }
        depth += line.text.matches('{').count() as isize;
        depth -= line.text.matches('}').count() as isize;
        if depth <= 0 && line.text.contains('}') {
            return Some((start?, line.offset + line.text.len()));
        }
    }
    start.map(|start| {
        (
            start,
            lines
                .last()
                .map(|line| line.offset + line.text.len())
                .unwrap_or(start),
        )
    })
}

fn mutation_offset(source: &str, binding: &str) -> Option<usize> {
    [
        ".push(", ".insert(", ".extend(", ".remove(", " += ", " -= ", " *= ", " /= ", " %= ",
    ]
    .iter()
    .filter_map(|method| {
        source
            .find(&format!("{binding}{method}"))
            .or_else(|| source.find(&format!("{binding}{}", spaced_method(method))))
    })
    .min()
}

fn spaced_method(method: &str) -> String {
    method
        .chars()
        .flat_map(|ch| {
            if ch == '.' || ch == '(' {
                vec![' ', ch, ' ']
            } else {
                vec![ch]
            }
        })
        .collect::<String>()
}

fn contains_ident(source: &str, ident: &str) -> bool {
    ident_offset(source, ident).is_some()
}

fn ident_offset(source: &str, ident: &str) -> Option<usize> {
    source.match_indices(ident).find_map(|(index, _)| {
        let before = source[..index].chars().next_back();
        let after = source[index + ident.len()..].chars().next();
        (!before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)).then_some(index)
    })
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}
