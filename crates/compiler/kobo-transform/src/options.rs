/// Transform-time knobs that affect ownership selection heuristics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformOptions {
    /// Byte threshold below which the small-struct clone heuristic may elide clones.
    pub small_struct_clone_threshold_bytes: usize,
    /// Fully-qualified type names that should be treated as `Copy`.
    pub copy_types: Vec<String>,
    /// Method names (or `Type::method`) that should be treated as `&mut self`.
    pub mutating_methods: Vec<String>,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            small_struct_clone_threshold_bytes: 128,
            copy_types: Vec::new(),
            mutating_methods: Vec::new(),
        }
    }
}
