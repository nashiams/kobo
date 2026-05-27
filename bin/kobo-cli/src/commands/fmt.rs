use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use kobo_driver::{run_codegen_pipeline, CodegenArtifacts};

use super::session::{build_session, render_diagnostics};

pub(super) fn cmd_fmt(file: &Path) -> anyhow::Result<()> {
    let original =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    if is_kobo_only_syntax(&original) {
        fs::write(file, format_kobo_only_source(&original))
            .with_context(|| format!("failed to write {}", file.display()))?;
        return Ok(());
    }

    let mut session = build_session(file, None)?;
    let artifacts = run_codegen_pipeline(&mut session, file).map_err(|()| {
        render_diagnostics(&session);
        anyhow::anyhow!("compilation failed")
    })?;
    // §3.3b: Render K-code warnings on success path too [R6-06].
    render_diagnostics(&session);
    run_rustfmt_file(&artifacts.rs_path)?;

    let formatted_source = rewrite_lossless_kobo_source(file, &artifacts)?;
    fs::write(file, formatted_source)
        .with_context(|| format!("failed to write {}", file.display()))?;

    Ok(())
}

fn run_rustfmt_file(path: &Path) -> anyhow::Result<()> {
    let rustfmt_output = Command::new("rustfmt")
        .arg(path)
        .output()
        .with_context(|| format!("failed to invoke rustfmt for {}", path.display()));

    let rustfmt_output = match rustfmt_output {
        Ok(output) => output,
        Err(error) if is_command_not_found(&error) => {
            anyhow::bail!("rustfmt is unavailable; install the Rust toolchain component first");
        }
        Err(error) => return Err(error),
    };

    if !rustfmt_output.status.success() {
        anyhow::bail!("rustfmt exited with a non-zero status");
    }

    Ok(())
}

fn rewrite_lossless_kobo_source(
    file: &Path,
    artifacts: &CodegenArtifacts,
) -> anyhow::Result<String> {
    let _ = artifacts.source_map.kobo_path();

    let original =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    if is_kobo_only_syntax(&original) {
        return Ok(format_kobo_only_source(&original));
    }

    // Formatting back-propagates from the original `.kobo` text.
    // Compiler-owned wrapper lines exist only in generated Rust, so they never
    // flow back into the user file through `kobo fmt`.
    rustfmt_original_kobo_source(file)
}

fn is_kobo_only_syntax(source: &str) -> bool {
    contains_word_outside_text(source, "ward")
        || source.contains("// kobo:invariant")
        || source.contains("// kobo:temporal")
}

fn format_kobo_only_source(source: &str) -> String {
    let expanded = expand_kobo_format_tokens(source);
    let mut formatted = String::new();
    let mut indent = 0usize;
    for raw_line in expanded.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "}" {
            indent = indent.saturating_sub(1);
        }
        formatted.push_str(&"    ".repeat(indent));
        formatted.push_str(line);
        formatted.push('\n');
        if line.ends_with('{') {
            indent += 1;
        }
    }
    formatted
}

fn expand_kobo_format_tokens(source: &str) -> String {
    let mut expanded = String::with_capacity(source.len() + 64);
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    let mut in_line_comment = false;
    while let Some(ch) = chars.next() {
        if in_line_comment {
            expanded.push(ch);
            if ch == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_string {
            expanded.push(ch);
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    expanded.push(next);
                }
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            expanded.push(ch);
            if let Some(next) = chars.next() {
                expanded.push(next);
            }
            in_line_comment = true;
            continue;
        }
        if ch == '"' {
            expanded.push(ch);
            in_string = true;
            continue;
        }
        match ch {
            '{' => expanded.push_str(" {\n"),
            '}' => expanded.push_str("\n}\n"),
            ';' => expanded.push_str(";\n"),
            _ => expanded.push(ch),
        }
    }
    expanded
}

fn contains_word_outside_text(source: &str, word: &str) -> bool {
    let bytes = source.as_bytes();
    let needle = word.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    while index < bytes.len() {
        if in_line_comment {
            if bytes[index] == b'\n' {
                in_line_comment = false;
            }
            index += 1;
            continue;
        }
        if in_string {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
            } else {
                if bytes[index] == b'"' {
                    in_string = false;
                }
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            in_line_comment = true;
            index += 2;
            continue;
        }
        if bytes[index] == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if index + needle.len() <= bytes.len() && &bytes[index..index + needle.len()] == needle {
            let end = index + needle.len();
            let before = if index == 0 {
                None
            } else {
                bytes.get(index - 1)
            };
            let after = bytes.get(end);
            if !before.is_some_and(is_ident_byte)
                && after.is_some_and(|byte| byte.is_ascii_whitespace())
            {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn is_ident_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || *byte == b'_'
}

fn rustfmt_original_kobo_source(file: &Path) -> anyhow::Result<String> {
    let source =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    let temp_path = rustfmt_temp_path(file);
    fs::write(&temp_path, &source)
        .with_context(|| format!("failed to stage {}", temp_path.display()))?;
    run_rustfmt_file(&temp_path)?;

    let formatted = fs::read_to_string(&temp_path)
        .with_context(|| format!("failed to read {}", temp_path.display()));
    let _ = fs::remove_file(&temp_path);

    formatted
}

fn rustfmt_temp_path(file: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "kobo".to_owned());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    std::env::temp_dir().join(format!("kobo-fmt-{stem}-{}-{nonce}.rs", std::process::id()))
}

fn is_command_not_found(error: &anyhow::Error) -> bool {
    error
        .root_cause()
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io_error| io_error.kind() == std::io::ErrorKind::NotFound)
}
