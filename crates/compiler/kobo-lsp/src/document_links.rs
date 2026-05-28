use serde_json::{json, Value};

use crate::navigation::source_map_metadata;
use crate::protocol::{default_document_range, protocol_range_json, ProtocolRustNavigation};

pub(crate) fn protocol_document_links(
    witness_path: Option<&str>,
    navigation: Option<&ProtocolRustNavigation>,
) -> Vec<Value> {
    let mut links = Vec::new();
    let link_range = navigation
        .map(|navigation| protocol_range_json(&navigation.source_range))
        .unwrap_or_else(default_document_range);
    if let Some(path) = witness_path {
        let mut link = json!({
            "range": link_range.clone(),
            "target": path,
            "tooltip": "Replay Kobo witness",
        });
        if let Some(navigation) = navigation {
            link["data"] = source_map_metadata(navigation);
        }
        links.push(link);
    }
    if let Some(navigation) = navigation {
        links.push(json!({
            "range": protocol_range_json(&navigation.source_range),
            "target": navigation.generated_uri,
            "tooltip": "Open generated Rust through source map",
            "data": source_map_metadata(navigation),
        }));
    }
    links
}
