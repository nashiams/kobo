use std::fs;
use std::path::{Path, PathBuf};

use crate::config::KoboConfig;

/// File-system errors emitted by the v0.1 driver.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("file not found: {path}")]
    NotFound { path: PathBuf },
    #[error("permission denied: {path}")]
    PermissionDenied { path: PathBuf },
    #[error("file is not valid UTF-8: {path}")]
    Utf8Error { path: PathBuf },
    #[error("{0}")]
    Other(String),
}

/// Reads a `.kobo` source file and returns its UTF-8 source text.
pub fn read_kobo_file(path: &Path) -> Result<String, IoError> {
    let bytes = fs::read(path).map_err(|source| io_error_from(source, path))?;
    String::from_utf8(bytes).map_err(|_| IoError::Utf8Error {
        path: path.to_path_buf(),
    })
}

/// Writes the generated Rust source to disk.
pub fn write_rs_file(path: &Path, content: &str) -> Result<(), IoError> {
    if let Some(parent_dir) = path.parent() {
        fs::create_dir_all(parent_dir).map_err(|source| io_error_from(source, path))?;
    }

    fs::write(path, content).map_err(|source| io_error_from(source, path))
}

pub fn write_map_file(path: &Path, content: &str) -> Result<(), IoError> {
    if let Some(parent_dir) = path.parent() {
        fs::create_dir_all(parent_dir).map_err(|source| io_error_from(source, path))?;
    }

    fs::write(path, content).map_err(|source| io_error_from(source, path))
}

/// Derives the default output `.rs` path from a `.kobo` input path.
pub fn rs_path_for(kobo_path: &Path) -> PathBuf {
    kobo_path.with_extension("rs")
}

/// Derives the actual output path, honoring `output_dir` when it is configured.
pub fn output_path_for(kobo_path: &Path, config: &KoboConfig) -> PathBuf {
    let default_output_path = rs_path_for(kobo_path);

    let Some(output_dir) = config.output_dir.as_ref() else {
        return default_output_path;
    };

    let output_file_name = default_output_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out.rs"));

    output_dir.join(output_file_name)
}

pub fn map_path_for(kobo_path: &Path, config: &KoboConfig) -> PathBuf {
    let default_map_path = kobo_path.with_extension("kobo.map");

    let Some(output_dir) = config.output_dir.as_ref() else {
        return default_map_path;
    };

    let output_file_name = default_map_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out.kobo.map"));

    output_dir.join(output_file_name)
}

fn io_error_from(source: std::io::Error, path: &Path) -> IoError {
    match source.kind() {
        std::io::ErrorKind::NotFound => IoError::NotFound {
            path: path.to_path_buf(),
        },
        std::io::ErrorKind::PermissionDenied => IoError::PermissionDenied {
            path: path.to_path_buf(),
        },
        _ => IoError::Other(source.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::config::KoboConfig;

    use super::{map_path_for, output_path_for, rs_path_for};

    #[test]
    fn rs_path_for_replaces_kobo_extension() {
        let output_path = rs_path_for(Path::new("src/demo.kobo"));
        assert_eq!(output_path, Path::new("src/demo.rs"));
    }

    #[test]
    fn output_path_for_uses_configured_output_dir() {
        let mut config = KoboConfig::default();
        config.output_dir = Some(Path::new("generated").to_path_buf());

        let output_path = output_path_for(Path::new("src/demo.kobo"), &config);
        assert_eq!(output_path, Path::new("generated/demo.rs"));
    }

    #[test]
    fn map_path_for_uses_kobo_map_suffix() {
        let mut config = KoboConfig::default();
        config.output_dir = Some(Path::new("generated").to_path_buf());

        let map_path = map_path_for(Path::new("src/demo.kobo"), &config);
        assert_eq!(map_path, Path::new("generated/demo.kobo.map"));
    }
}
