use std::path::PathBuf;

/// Errors that can occur during the kobo driver pipeline.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("parse failed for {file}: {message}")]
    ParseFailed { file: PathBuf, message: String },

    #[error("transform failed for {file}: {message}")]
    TransformFailed { file: PathBuf, message: String },

    #[error("cargo build failed (exit code {exit_code}): {stderr}")]
    CargoBuildFailed { stderr: String, exit_code: i32 },

    #[error("Cargo.toml generation failed: {message}")]
    CargoTomlGenFailed { message: String },

    #[error("circular dependency detected: {}", format_cycle(cycle))]
    CircularDependency { cycle: Vec<PathBuf> },
}

fn format_cycle(cycle: &[PathBuf]) -> String {
    cycle
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_displays() {
        let err = DriverError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        let msg = err.to_string();
        assert!(msg.contains("file not found"), "got: {msg}");
    }

    #[test]
    fn parse_failed_displays_file_and_message() {
        let err = DriverError::ParseFailed {
            file: PathBuf::from("src/main.kobo"),
            message: "unexpected token".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("main.kobo"), "got: {msg}");
        assert!(msg.contains("unexpected token"), "got: {msg}");
    }

    #[test]
    fn transform_failed_displays() {
        let err = DriverError::TransformFailed {
            file: PathBuf::from("lib.kobo"),
            message: "scope error".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("lib.kobo"), "got: {msg}");
        assert!(msg.contains("scope error"), "got: {msg}");
    }

    #[test]
    fn cargo_build_failed_displays_exit_code() {
        let err = DriverError::CargoBuildFailed {
            stderr: "error[E0425]: cannot find value".to_string(),
            exit_code: 101,
        };
        let msg = err.to_string();
        assert!(msg.contains("101"), "got: {msg}");
        assert!(msg.contains("E0425"), "got: {msg}");
    }

    #[test]
    fn cargo_toml_gen_failed_displays() {
        let err = DriverError::CargoTomlGenFailed {
            message: "invalid name".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("invalid name"), "got: {msg}");
    }

    #[test]
    fn circular_dependency_displays_cycle() {
        let err = DriverError::CircularDependency {
            cycle: vec![
                PathBuf::from("a.kobo"),
                PathBuf::from("b.kobo"),
                PathBuf::from("a.kobo"),
            ],
        };
        let msg = err.to_string();
        assert!(msg.contains("a.kobo -> b.kobo -> a.kobo"), "got: {msg}");
    }
}
