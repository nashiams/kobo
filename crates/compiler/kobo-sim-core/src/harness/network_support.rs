use kobo_ir::{ScenarioOpKind, ScenarioProgram};

use crate::core::ScenarioOptions;
use crate::error::Result;

use super::events::event_print_statements;

pub(super) fn network_support_source(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> Result<String> {
    let mut methods = Vec::new();
    for operation in &program.operations {
        if let ScenarioOpKind::NetworkEvent { action } = &operation.kind {
            if methods.iter().any(|existing| existing == action) {
                continue;
            }
            methods.push(action.clone());
        }
    }
    for default_method in ["send", "delay", "reorder", "receive", "drop_message"] {
        if !methods.iter().any(|existing| existing == default_method) {
            methods.push(default_method.to_owned());
        }
    }
    let mut source = String::from(
        r#"
fn __kobo_network_loopback() {
    let Ok(socket) = std::net::UdpSocket::bind("127.0.0.1:0") else {
        return;
    };
    let Ok(address) = socket.local_addr() else {
        return;
    };
    let _ = socket.set_nonblocking(true);
    let _ = socket.send_to(b"kobo-network-frame", address);
    let mut buffer = [0_u8; 64];
    let _ = socket.recv_from(&mut buffer);
}

"#,
    );
    source.push_str("impl __KoboWardNetwork {\n");
    for method in methods {
        source.push_str("    fn ");
        source.push_str(&method);
        source.push_str("<T>(&self, _value: T) {\n        ");
        source.push_str(network_runtime_statement(&method));
        source.push_str("\n        ");
        source.push_str(&event_print_statements(
            &crate::network::harness_events_for_action(&method, options.seed),
        )?);
        source.push_str("\n    }\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn network_runtime_statement(method: &str) -> &'static str {
    match normalized_network_method(method).as_str() {
        "send" | "receive" => "__kobo_network_loopback();",
        _ => "",
    }
}

fn normalized_network_method(method: &str) -> String {
    method
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}
