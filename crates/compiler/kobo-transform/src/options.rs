/// Transform-time knobs that affect ownership selection heuristics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransformOptions {
    /// Byte threshold below which the small-struct clone heuristic may elide clones.
    pub small_struct_clone_threshold_bytes: usize,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            small_struct_clone_threshold_bytes: 128,
        }
    }
}
