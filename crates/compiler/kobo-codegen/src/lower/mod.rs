mod binding;
mod plan;
mod rewrite;
mod scope;
mod support;

use kobo_ir::SolutionMap;
use kobo_parser::KoboFile;

use self::plan::LoweringPlan;
use self::rewrite::Lowerer;

pub use self::plan::LoweringSite;

pub struct LoweredFile {
    pub file: syn::File,
    pub sites: Vec<LoweringSite>,
}

/// Formatted Rust lowering driven by the frozen KIR plus any solved ownership overrides.
pub fn lower(kir: &kobo_ir::Kir, ast: &KoboFile, solution: &SolutionMap) -> LoweredFile {
    let mut file = ast.inner.clone();
    let plan = LoweringPlan::from_kir(ast, kir, solution);
    let mut lowerer = Lowerer::new(ast, &plan);
    lowerer.lower_items(&mut file.items);
    plan.insert_support_items(&mut file);

    LoweredFile {
        file,
        sites: plan.annotation_sites().to_vec(),
    }
}
