use std::path::PathBuf;

// --- Newtype IDs ---

/// Stable identifier for a node in the Kobo AST.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct KoboAstNodeId(pub u32);

/// Stable identifier for a KIR node.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct KirNodeId(pub u32);

/// Stable identifier for a CFG basic block (used by the async pass in v0.7).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct CfgBlockId(pub u32);

/// Index into `CompileSession.file_set`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct FileId(pub u32);

// --- NodeIdGen ---

/// Counter-based ID generator. Owned by `CompileSession`; passed through the pipeline.
///
/// IDs are monotonically increasing within a session. Never reused.
#[derive(Debug, Default)]
pub struct NodeIdGen {
    next: u32,
}

impl NodeIdGen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_ast_id(&mut self) -> KoboAstNodeId {
        let id = self.next;
        self.next += 1;
        KoboAstNodeId(id)
    }

    pub fn next_kir_id(&mut self) -> KirNodeId {
        let id = self.next;
        self.next += 1;
        KirNodeId(id)
    }
}

// --- FileSet ---

/// A single source file registered with the session.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub source: String,
}

/// All source files registered during a compile session.
#[derive(Debug, Default)]
pub struct FileSet {
    files: Vec<FileEntry>,
}

impl FileSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a source file and returns its `FileId`.
    pub fn register(&mut self, path: PathBuf, source: String) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(FileEntry { path, source });
        id
    }

    pub fn get(&self, id: FileId) -> Option<&FileEntry> {
        self.files.get(id.0 as usize)
    }

    pub fn iter_files(&self) -> impl Iterator<Item = (FileId, &FileEntry)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, entry)| (FileId(i as u32), entry))
    }
}

// --- Id counter table (for testing clarity) ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_are_unique_and_monotonic() {
        let mut gen = NodeIdGen::new();
        let a = gen.next_ast_id();
        let b = gen.next_ast_id();
        assert_eq!(a.0, 0);
        assert_eq!(b.0, 1);
        assert!(a < b);

        let k0 = gen.next_kir_id();
        let k1 = gen.next_kir_id();
        assert_eq!(k0.0, 2);
        assert_eq!(k1.0, 3);
        assert!(k0 < k1);
    }
}
