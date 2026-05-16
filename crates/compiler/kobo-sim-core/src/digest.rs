use crate::core::{ScenarioEvent, ScenarioOperation};

pub fn stable_hash(material: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in material.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub fn events_hash(events: &[ScenarioEvent]) -> String {
    let mut material = String::new();
    for event in events {
        material.push_str(&event.kind);
        material.push('|');
        if let Some(label) = event.label.as_deref() {
            material.push_str(label);
        }
        material.push('|');
        if let Some(value) = event.value {
            material.push_str(&value.to_string());
        }
        material.push('\n');
    }
    stable_hash(&material)
}

pub fn operations_hash(operations: &[ScenarioOperation]) -> String {
    let mut material = String::new();
    for operation in operations {
        material.push_str(&format!("{operation:?}"));
        material.push('\n');
    }
    stable_hash(&material)
}
