//! Stage: Persistent decisions cache (.kobo/decisions.toml).
//!
//! Stores user-reviewed decisions for persistence across builds.
//! Decisions can be locked (user-approved) or provisional (auto-resolved).

use std::collections::BTreeMap;
use std::path::Path;

use kobo_ir::OwnershipTier;

/// A persisted decision entry.
#[derive(Clone, Debug)]
pub struct PersistedDecision {
    pub binding_name: String,
    pub tier: OwnershipTier,
    pub locked: bool,
    pub reviewed_by: Option<String>,
    pub comment: Option<String>,
}

/// In-memory representation of decisions.toml.
#[derive(Clone, Debug, Default)]
pub struct DecisionsStore {
    entries: BTreeMap<String, PersistedDecision>,
}

impl DecisionsStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from a TOML file path. Returns empty store on missing file.
    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Self::new();
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::new(),
        };
        Self::parse(&content)
    }

    /// Parse from TOML string.
    fn parse(content: &str) -> Self {
        let mut store = Self::new();
        // Simple line-based parser for [binding.NAME] sections.
        let mut current_name: Option<String> = None;
        let mut current_tier: Option<OwnershipTier> = None;
        let mut current_locked = false;
        let mut current_reviewed: Option<String> = None;
        let mut current_comment: Option<String> = None;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("[binding.") && trimmed.ends_with(']') {
                // Flush previous.
                if let (Some(name), Some(tier)) = (current_name.take(), current_tier.take()) {
                    store.entries.insert(
                        name.clone(),
                        PersistedDecision {
                            binding_name: name,
                            tier,
                            locked: current_locked,
                            reviewed_by: current_reviewed.take(),
                            comment: current_comment.take(),
                        },
                    );
                }
                let name = trimmed
                    .trim_start_matches("[binding.")
                    .trim_end_matches(']')
                    .to_owned();
                current_name = Some(name);
                current_tier = None;
                current_locked = false;
                current_reviewed = None;
                current_comment = None;
            } else if let Some(eq_pos) = trimmed.find('=') {
                let key = trimmed[..eq_pos].trim();
                let val = trimmed[eq_pos + 1..].trim().trim_matches('"');
                match key {
                    "tier" => current_tier = parse_tier(val),
                    "locked" => current_locked = val == "true",
                    "reviewed_by" => current_reviewed = Some(val.to_owned()),
                    "comment" => current_comment = Some(val.to_owned()),
                    _ => {}
                }
            }
        }
        // Flush last.
        if let (Some(name), Some(tier)) = (current_name, current_tier) {
            store.entries.insert(
                name.clone(),
                PersistedDecision {
                    binding_name: name,
                    tier,
                    locked: current_locked,
                    reviewed_by: current_reviewed,
                    comment: current_comment,
                },
            );
        }
        store
    }

    /// Get a decision by binding name.
    pub fn get(&self, name: &str) -> Option<&PersistedDecision> {
        self.entries.get(name)
    }

    /// Insert or update a decision.
    pub fn set(&mut self, decision: PersistedDecision) {
        self.entries.insert(decision.binding_name.clone(), decision);
    }

    /// Check if a binding is locked.
    pub fn is_locked(&self, name: &str) -> bool {
        self.entries.get(name).map(|d| d.locked).unwrap_or(false)
    }

    /// Iterate all entries.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &PersistedDecision)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("# Kobo ownership decisions — managed by `kobo migrate`\n\n");
        for (name, d) in &self.entries {
            out.push_str(&format!("[binding.{}]\n", name));
            out.push_str(&format!("tier = \"{:?}\"\n", d.tier));
            out.push_str(&format!("locked = {}\n", d.locked));
            if let Some(ref r) = d.reviewed_by {
                out.push_str(&format!("reviewed_by = \"{}\"\n", r));
            }
            if let Some(ref c) = d.comment {
                out.push_str(&format!("comment = \"{}\"\n", c));
            }
            out.push('\n');
        }
        out
    }

    /// Save to file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_toml())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn parse_tier(s: &str) -> Option<OwnershipTier> {
    match s {
        "PlainOwned" => Some(OwnershipTier::PlainOwned),
        "BoxOwned" => Some(OwnershipTier::BoxOwned),
        "RcShared" => Some(OwnershipTier::RcShared),
        "ArcShared" => Some(OwnershipTier::ArcShared),
        "RcMutShared" => Some(OwnershipTier::RcMutShared),
        "ArcMutShared" => Some(OwnershipTier::ArcMutShared),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store() {
        let store = DecisionsStore::new();
        assert!(store.is_empty());
        assert!(store.get("x").is_none());
    }

    #[test]
    fn round_trip_toml() {
        let mut store = DecisionsStore::new();
        store.set(PersistedDecision {
            binding_name: "my_var".into(),
            tier: OwnershipTier::RcShared,
            locked: true,
            reviewed_by: Some("dev".into()),
            comment: Some("looks good".into()),
        });
        let toml = store.to_toml();
        let reloaded = DecisionsStore::parse(&toml);
        assert_eq!(reloaded.len(), 1);
        let d = reloaded.get("my_var").unwrap();
        assert_eq!(d.tier, OwnershipTier::RcShared);
        assert!(d.locked);
    }

    #[test]
    fn locked_check() {
        let mut store = DecisionsStore::new();
        store.set(PersistedDecision {
            binding_name: "a".into(),
            tier: OwnershipTier::PlainOwned,
            locked: true,
            reviewed_by: None,
            comment: None,
        });
        store.set(PersistedDecision {
            binding_name: "b".into(),
            tier: OwnershipTier::PlainOwned,
            locked: false,
            reviewed_by: None,
            comment: None,
        });
        assert!(store.is_locked("a"));
        assert!(!store.is_locked("b"));
        assert!(!store.is_locked("c")); // nonexistent
    }
}
