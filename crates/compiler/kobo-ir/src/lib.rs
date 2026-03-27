mod kir;
mod node_id;
mod ownership;
mod resource;
mod solution_map;
mod span;

pub use kir::{Kir, KirNode};
pub use kir::{BorrowKind, NodeKind, UseKind};
pub use node_id::{
    CfgBlockId, FileEntry, FileId, FileSet, FileSetBuilder, KirNodeId, KoboAstNodeId, NodeIdGen,
};
pub use ownership::OwnershipTier;
pub use resource::ResourceKind;
pub use solution_map::SolutionMap;
pub use span::KoboSpan;
