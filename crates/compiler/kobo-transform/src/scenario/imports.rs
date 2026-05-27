use super::*;

pub(super) fn collect_use_crate_aliases(file: &File) -> ImportMap {
    let mut imports = HashMap::new();
    for item in &file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        collect_use_tree_aliases(&item_use.tree, Vec::new(), &mut imports);
    }
    imports
}

pub(super) fn collect_use_tree_aliases(
    tree: &UseTree,
    prefix: Vec<String>,
    imports: &mut ImportMap,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_tree_aliases(&path.tree, next, imports);
        }
        UseTree::Name(name) => {
            let mut full_path = prefix;
            full_path.push(name.ident.to_string());
            imports.insert(name.ident.to_string(), full_path);
        }
        UseTree::Rename(rename) => {
            let mut full_path = prefix;
            full_path.push(rename.ident.to_string());
            imports.insert(rename.rename.to_string(), full_path);
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                collect_use_tree_aliases(tree, prefix.clone(), imports);
            }
        }
        UseTree::Glob(_) => {}
    }
}
