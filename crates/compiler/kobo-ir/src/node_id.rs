use std::path::{Path, PathBuf};

// --- Newtype IDs ---

/// Stable identifier for a node in the Kobo AST.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct KoboAstNodeId(pub u32);

/// Stable identifier for a KIR node.
#[derive(
    Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct KirNodeId(pub u32);

/// Stable identifier for a CFG basic block (used by the async pass in v0.7).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct CfgBlockId(pub u32);

/// Index into `CompileSession.file_set`.
#[derive(
    Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
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
    line_starts: Vec<usize>,
}

/// All source files registered during a compile session.
#[derive(Debug, Default)]
pub struct FileSet {
    files: Vec<FileEntry>,
}

/// Construction-time builder for a `FileSet`.
#[derive(Debug, Default)]
pub struct FileSetBuilder {
    file_set: FileSet,
}

impl FileSet {
    pub fn new() -> Self {
        Self::default()
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

impl FileSetBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, path: PathBuf, source: String) -> FileId {
        self.file_set.register(path, source)
    }

    pub fn as_file_set(&self) -> &FileSet {
        &self.file_set
    }

    pub fn finish(self) -> FileSet {
        self.file_set
    }
}

impl FileEntry {
    fn new(path: PathBuf, source: String) -> Self {
        let line_starts = build_line_starts(&source);
        Self {
            path,
            source,
            line_starts,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Resolves a byte offset to a 1-based `(line, column)` pair.
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let clamped_offset = (offset as usize).min(self.source.len());
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= clamped_offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index];

        (
            line_index + 1,
            clamped_offset.saturating_sub(line_start) + 1,
        )
    }

    /// Returns the text for a 1-based line number without its trailing newline.
    pub fn line_text(&self, line: usize) -> Option<&str> {
        if line == 0 {
            return None;
        }

        let start = *self.line_starts.get(line - 1)?;
        let next_start = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.source.len());
        let raw_line = self
            .source
            .get(start..next_start.saturating_sub(1))
            .or_else(|| self.source.get(start..next_start))?;

        Some(raw_line.strip_suffix('\r').unwrap_or(raw_line))
    }

    pub fn snippet(&self, span: crate::span::KoboSpan) -> Option<&str> {
        if span.start > span.end || span.end as usize > self.source.len() {
            return None;
        }

        self.source.get(span.start as usize..span.end as usize)
    }
}

fn build_line_starts(source: &str) -> Vec<usize> {
    let mut line_starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(index + 1);
        }
    }

    line_starts
}

impl FileSet {
    fn register(&mut self, path: PathBuf, source: String) -> FileId {
        let file_id = FileId(self.files.len() as u32);
        self.files.push(FileEntry::new(path, source));
        file_id
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

    #[test]
    fn file_entry_resolves_lines_columns_and_snippets() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file(
            PathBuf::from("demo.kobo"),
            "alpha\nbeta\r\ngamma".to_owned(),
        );
        let file_set = file_set_builder.finish();
        let file = file_set.get(file_id).expect("registered file should exist");

        assert_eq!(file.line_col(0), (1, 1));
        assert_eq!(file.line_col(7), (2, 2));
        assert_eq!(file.line_text(1), Some("alpha"));
        assert_eq!(file.line_text(2), Some("beta"));
        assert_eq!(
            file.snippet(crate::span::KoboSpan::new(6, 10, file_id)),
            Some("beta")
        );
    }
}
