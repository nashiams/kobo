use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use kobo_driver::{load_config_for, run_check_pipeline, CompileSession};
use kobo_errors::DiagnosticLspPayload;
use kobo_ir::{GuaranteePolicy, GuaranteeProfile};

type LspDocuments = BTreeMap<String, LspDocument>;

#[derive(Clone, Debug)]
struct LspDocument {
    text: String,
    witness_path: Option<String>,
}

#[derive(Clone, Debug)]
struct WitnessCandidate {
    modified: SystemTime,
    path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("kobo-lsp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.iter().any(|arg| arg == "--stdio") {
        return run_stdio();
    }

    let Some(file) = diagnostics_file_arg(&args) else {
        eprintln!("usage: kobo-lsp --stdio | --version | --diagnostics FILE");
        std::process::exit(2);
    };

    publish_diagnostics(&file)
}

fn run_stdio() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut first_line = String::new();
    if reader.read_line(&mut first_line)? == 0 {
        emit_protocol_value(
            &kobo_lsp::initialize_response(serde_json::Value::Null),
            false,
        );
        return Ok(());
    }
    let mut documents = LspDocuments::new();

    if !line_is_content_length(&first_line) {
        let mut input = first_line;
        reader.read_to_string(&mut input)?;
        for value in parse_json_rpc_inputs(&input) {
            if process_json_rpc_value(value, false, &mut documents) {
                break;
            }
        }
        return Ok(());
    }

    let mut header_line = first_line;
    loop {
        let Some(value) = read_framed_json_rpc_value(&mut reader, header_line)? else {
            break;
        };
        if process_json_rpc_value(value, true, &mut documents) {
            break;
        }
        header_line = String::new();
        if reader.read_line(&mut header_line)? == 0 {
            break;
        }
        while header_line.trim().is_empty() {
            header_line.clear();
            if reader.read_line(&mut header_line)? == 0 {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn process_json_rpc_value(
    value: serde_json::Value,
    framed: bool,
    documents: &mut LspDocuments,
) -> bool {
    match value["method"].as_str() {
        Some("initialize") => {
            let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
            emit_protocol_value(&kobo_lsp::initialize_response(id), framed);
        }
        Some("shutdown") => {
            let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
            emit_protocol_value(&json_rpc_response(id, serde_json::Value::Null), framed);
        }
        Some("exit") => return true,
        Some("textDocument/didOpen") => {
            if let Some((uri, document, notification)) = open_document_notification(&value) {
                documents.insert(uri, document);
                emit_protocol_value(&notification, framed);
            }
        }
        Some("textDocument/hover") => {
            if let Some(response) = document_feature_response(&value, documents, "hover") {
                emit_protocol_value(&response, framed);
            }
        }
        Some("textDocument/codeAction") => {
            if let Some(response) = document_feature_response(&value, documents, "codeActions") {
                emit_protocol_value(&response, framed);
            }
        }
        Some("textDocument/documentLink") => {
            if let Some(response) = document_feature_response(&value, documents, "documentLinks") {
                emit_protocol_value(&response, framed);
            }
        }
        Some("textDocument/definition") => {
            if let Some(response) = definition_response(&value, documents) {
                emit_protocol_value(&response, framed);
            }
        }
        Some("kobo/runnables") => {
            if let Some(response) = document_feature_response(&value, documents, "runnables") {
                emit_protocol_value(&response, framed);
            }
        }
        _ => {}
    }
    false
}

fn open_document_notification(
    value: &serde_json::Value,
) -> Option<(String, LspDocument, serde_json::Value)> {
    let document = &value["params"]["textDocument"];
    let uri = document["uri"].as_str()?;
    let text = document["text"].as_str()?;
    let witness_path = discover_witness_for_uri(uri, text);
    let snapshot = kobo_lsp::protocol_document_snapshot(uri, text, witness_path.as_deref());
    let notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": snapshot["diagnostics"].clone(),
        },
    });
    let document = LspDocument {
        text: text.to_owned(),
        witness_path,
    };
    Some((uri.to_owned(), document, notification))
}

fn document_feature_response(
    value: &serde_json::Value,
    documents: &LspDocuments,
    key: &str,
) -> Option<serde_json::Value> {
    let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let uri = value["params"]["textDocument"]["uri"].as_str()?;
    let document = documents.get(uri);
    let text = document
        .map(|document| document.text.as_str())
        .unwrap_or_default();
    let witness_path = document.and_then(|document| document.witness_path.as_deref());
    let snapshot = kobo_lsp::protocol_document_snapshot(uri, text, witness_path);
    Some(json_rpc_response(id, snapshot[key].clone()))
}

fn definition_response(
    value: &serde_json::Value,
    documents: &LspDocuments,
) -> Option<serde_json::Value> {
    let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let uri = value["params"]["textDocument"]["uri"].as_str()?;
    let document = documents.get(uri);
    let text = document
        .map(|document| document.text.as_str())
        .unwrap_or_default();
    let witness_path = document.and_then(|document| document.witness_path.as_deref());
    let snapshot = kobo_lsp::protocol_document_snapshot(uri, text, witness_path);
    Some(json_rpc_response(id, snapshot["definitions"].clone()))
}

fn discover_witness_for_uri(uri: &str, source: &str) -> Option<String> {
    let document_path = file_uri_to_path(uri)?;
    latest_witness_for_document(&document_path, source)
        .map(|path| path.to_string_lossy().to_string())
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let raw_path = uri.strip_prefix("file://")?;
    let decoded = percent_decode_uri_path(raw_path)?;
    let windows_drive_path =
        decoded.starts_with('/') && decoded.as_bytes().get(2).is_some_and(|byte| *byte == b':');
    let path = if windows_drive_path {
        decoded.get(1..).unwrap_or(decoded.as_str())
    } else {
        decoded.as_str()
    };
    Some(PathBuf::from(path))
}

fn percent_decode_uri_path(path: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn latest_witness_for_document(document_path: &Path, source: &str) -> Option<PathBuf> {
    let start = document_path.parent()?;
    let source_hash = kobo_sim_core::digest::stable_hash(source);
    for ancestor in start.ancestors() {
        let witness_dir = ancestor.join(".kobo").join("witnesses");
        if let Some(witness_path) =
            latest_witness_in_dir(&witness_dir, ancestor, document_path, &source_hash)
        {
            return Some(witness_path);
        }
    }
    None
}

fn latest_witness_in_dir(
    witness_dir: &Path,
    witness_root: &Path,
    document_path: &Path,
    source_hash: &str,
) -> Option<PathBuf> {
    let entries = std::fs::read_dir(witness_dir).ok()?;
    let mut latest: Option<WitnessCandidate> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("kwit") {
            continue;
        }
        if !witness_matches_document(&path, witness_root, document_path, source_hash) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        let should_replace = match latest.as_ref() {
            Some(latest) => {
                modified > latest.modified
                    || (modified == latest.modified
                        && path.to_string_lossy() > latest.path.to_string_lossy())
            }
            None => true,
        };
        if should_replace {
            latest = Some(WitnessCandidate { modified, path });
        }
    }
    latest.map(|candidate| candidate.path)
}

fn witness_matches_document(
    path: &Path,
    witness_root: &Path,
    document_path: &Path,
    source_hash: &str,
) -> bool {
    let Ok(source) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&source) else {
        return false;
    };
    if value
        .pointer("/source/hash")
        .and_then(serde_json::Value::as_str)
        != Some(source_hash)
    {
        return false;
    }
    let document = normalized_path_text(&document_path.to_string_lossy());
    let relative_document = document_path
        .strip_prefix(witness_root)
        .ok()
        .map(|path| normalized_path_text(&path.to_string_lossy()));
    let source_path_matches = value
        .pointer("/source/path")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|source_path| {
            normalized_path_eq(&document, source_path)
                || relative_document
                    .as_deref()
                    .is_some_and(|relative| normalized_path_eq(relative, source_path))
        });
    let target_path_matches = value
        .get("target")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|target| {
            normalized_target_starts_with(&document, target)
                || relative_document
                    .as_deref()
                    .is_some_and(|relative| normalized_target_starts_with(relative, target))
        });
    source_path_matches || target_path_matches
}

fn normalized_path_eq(document: &str, candidate: &str) -> bool {
    document.eq_ignore_ascii_case(&normalized_path_text(candidate))
}

fn normalized_target_starts_with(document: &str, target: &str) -> bool {
    let normalized_target = normalized_path_text(target);
    normalized_target
        .get(..document.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(document))
        && normalized_target
            .as_bytes()
            .get(document.len())
            .is_some_and(|byte| *byte == b':')
}

fn normalized_path_text(path: &str) -> String {
    path.replace('\\', "/")
}

fn json_rpc_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn emit_protocol_value(value: &serde_json::Value, framed: bool) {
    let body = value.to_string();
    if framed {
        print!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    } else {
        println!("{body}");
    }
    let _ = std::io::stdout().flush();
}

fn parse_json_rpc_inputs(input: &str) -> Vec<serde_json::Value> {
    let framed = parse_framed_json_rpc_inputs(input);
    if !framed.is_empty() {
        return framed;
    }
    let mut values = Vec::new();
    for line in input
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
    {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            values.push(value);
        }
    }
    if values.is_empty() {
        if let Some(start) = input.find('{') {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&input[start..]) {
                values.push(value);
            }
        }
    }
    values
}

fn parse_framed_json_rpc_inputs(input: &str) -> Vec<serde_json::Value> {
    let mut values = Vec::new();
    let bytes = input.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let Some(header_start) = find_case_insensitive(bytes, cursor, b"content-length:") else {
            break;
        };
        let Some(header_end) = find_header_end(bytes, header_start) else {
            break;
        };
        let header = &input[header_start..header_end];
        let Some(length) = content_length(header) else {
            cursor = header_end + 1;
            continue;
        };
        let body_start = if bytes.get(header_end) == Some(&b'\r') {
            header_end + 4
        } else {
            header_end + 2
        };
        let body_end = body_start.saturating_add(length);
        let Some(body) = input.get(body_start..body_end) else {
            break;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            values.push(value);
        }
        cursor = body_end;
    }
    values
}

fn read_framed_json_rpc_value(
    reader: &mut impl BufRead,
    first_header_line: String,
) -> anyhow::Result<Option<serde_json::Value>> {
    if !line_is_content_length(&first_header_line) {
        return Ok(None);
    }
    let mut header = first_header_line;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        header.push_str(&line);
        if line == "\r\n" || line == "\n" {
            break;
        }
    }
    let Some(length) = content_length(&header) else {
        return Ok(None);
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice::<serde_json::Value>(&body).ok())
}

fn line_is_content_length(line: &str) -> bool {
    line.split_once(':')
        .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
}

fn find_case_insensitive(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| {
            window
                .iter()
                .zip(needle)
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
        })
        .map(|relative| from + relative)
}

fn find_header_end(bytes: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|relative| from + relative)
        .or_else(|| {
            bytes
                .get(from..)?
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|relative| from + relative)
        })
}

fn content_length(header: &str) -> Option<usize> {
    header.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

fn diagnostics_file_arg(args: &[String]) -> Option<PathBuf> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--diagnostics" {
            return args.get(index + 1).map(PathBuf::from);
        }
    }
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
}

fn publish_diagnostics(file: &Path) -> anyhow::Result<()> {
    let mut session = build_lsp_session(file)?;
    let _ = run_check_pipeline(&mut session, file);

    for diagnostic in session.visible_diagnostics() {
        let payload = DiagnosticLspPayload::from_diagnostic(session.file_set(), diagnostic);
        println!(
            "{}",
            serde_json::to_string(&payload).expect("LSP diagnostic payload should serialize")
        );
    }
    Ok(())
}

fn build_lsp_session(file: &Path) -> anyhow::Result<CompileSession> {
    let workspace_root = find_workspace_root(file)?;
    let crate_dir = find_crate_dir(file, &workspace_root);
    let mut config = load_config_for(&crate_dir, &workspace_root)
        .with_context(|| format!("failed to load config for {}", file.display()))?;
    config.guarantee_policy = GuaranteePolicy::for_profile(GuaranteeProfile::Checked);
    let mut session = CompileSession::new(config);
    session.cli_profile_override = true;
    Ok(session)
}

fn find_workspace_root(file: &Path) -> anyhow::Result<PathBuf> {
    let search_root = input_directory(file)?;

    let mut workspace_root = None;
    for ancestor in search_root.ancestors() {
        if has_workspace_marker(ancestor) {
            workspace_root = Some(ancestor.to_path_buf());
        }
    }

    if let Some(workspace_root) = workspace_root {
        return Ok(workspace_root);
    }

    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    for ancestor in current_dir.ancestors() {
        if has_workspace_marker(ancestor) {
            return Ok(ancestor.to_path_buf());
        }
    }

    input_directory(file)
}

fn find_crate_dir(file: &Path, workspace_root: &Path) -> PathBuf {
    let Ok(search_root) = input_directory(file) else {
        return workspace_root.to_path_buf();
    };

    for ancestor in search_root.ancestors() {
        if ancestor == workspace_root {
            break;
        }

        if has_workspace_marker(ancestor) {
            return ancestor.to_path_buf();
        }
    }

    workspace_root.to_path_buf()
}

fn has_workspace_marker(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() || path.join("Kobo.toml").is_file()
}

fn input_directory(file: &Path) -> anyhow::Result<PathBuf> {
    let absolute_file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(file)
    };

    Ok(absolute_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(absolute_file))
}
