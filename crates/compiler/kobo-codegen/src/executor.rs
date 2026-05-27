use std::collections::HashMap;

/// Which async executor to use for the generated code.
///
/// Selection priority [Invariant k006x_async_executor]:
/// 1. If `[dependencies]` contains "tokio" → Tokio
/// 2. If `[dependencies]` contains "async-std" → AsyncStd
/// 3. If no executor dependency → None (emit K0062 warning)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorChoice {
    Tokio,
    AsyncStd,
    /// No executor found — emit K0062 warning.
    None,
}

/// Determines the spawn strategy for a given async block.
///
/// If all captured bindings are Send → TokioSpawn.
/// If any captured binding is !Send → LocalSet + spawn_local.
/// If undecidable → FallbackLocal + K0060 warning.
///
/// This enum is declared but not yet dispatched. `select_executor()`
/// handles executor *detection*; `SpawnStrategy` handles per-block *dispatch*
/// which requires Send analysis per spawn site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SpawnStrategy {
    /// All bindings are Send — use tokio::spawn.
    TokioSpawn,
    /// Some bindings are !Send — use LocalSet + spawn_local.
    LocalSet,
    /// Cannot determine — emit K0060 warning, fallback to LocalSet.
    FallbackLocal,
}

/// Select the async executor from the project's direct dependencies.
///
/// Only checks direct dependencies.
/// Transitive dependency detection is a future enhancement.
pub fn select_executor(dependencies: &HashMap<String, toml::Value>) -> ExecutorChoice {
    if dependencies.contains_key("tokio") {
        ExecutorChoice::Tokio
    } else if dependencies.contains_key("async-std") {
        ExecutorChoice::AsyncStd
    } else {
        ExecutorChoice::None
    }
}

/// Returns the executor attribute string to prepend to `async fn main()`.
///
/// For non-main async functions, returns `None` (no attribute needed).
/// For `ExecutorChoice::None`, returns `None` (K0062 emitted separately).
pub fn executor_attribute(choice: ExecutorChoice, is_main: bool) -> Option<&'static str> {
    if !is_main {
        return None;
    }
    match choice {
        ExecutorChoice::Tokio => Some("#[tokio::main]"),
        ExecutorChoice::AsyncStd => Some("#[async_std::main]"),
        ExecutorChoice::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: tokio in dependencies → Tokio selected.
    #[test]
    fn tokio_detected() {
        let deps = HashMap::from([("tokio".to_string(), toml::Value::String("1".into()))]);
        assert_eq!(select_executor(&deps), ExecutorChoice::Tokio);
    }

    /// Invariant: async-std in dependencies → AsyncStd selected.
    #[test]
    fn async_std_detected() {
        let deps = HashMap::from([("async-std".to_string(), toml::Value::String("1".into()))]);
        assert_eq!(select_executor(&deps), ExecutorChoice::AsyncStd);
    }

    /// Invariant: no executor dependency → None.
    #[test]
    fn no_executor() {
        let deps = HashMap::new();
        assert_eq!(select_executor(&deps), ExecutorChoice::None);
    }

    /// Invariant: both tokio and async-std → tokio wins (priority 1).
    #[test]
    fn tokio_wins_over_async_std() {
        let deps = HashMap::from([
            ("tokio".to_string(), toml::Value::String("1".into())),
            ("async-std".to_string(), toml::Value::String("1".into())),
        ]);
        assert_eq!(select_executor(&deps), ExecutorChoice::Tokio);
    }

    /// Invariant: tokio with table value (features) is still detected.
    #[test]
    fn tokio_with_features() {
        let mut table = toml::map::Map::new();
        table.insert("version".to_string(), toml::Value::String("1".into()));
        table.insert(
            "features".to_string(),
            toml::Value::Array(vec![toml::Value::String("full".into())]),
        );
        let deps = HashMap::from([("tokio".to_string(), toml::Value::Table(table))]);
        assert_eq!(select_executor(&deps), ExecutorChoice::Tokio);
    }

    /// Invariant: non-main async fn → no executor attribute.
    #[test]
    fn non_main_no_attribute() {
        assert_eq!(executor_attribute(ExecutorChoice::Tokio, false), None);
    }

    /// Invariant: main + tokio → #[tokio::main] attribute.
    #[test]
    fn main_tokio_attribute() {
        assert_eq!(
            executor_attribute(ExecutorChoice::Tokio, true),
            Some("#[tokio::main]")
        );
    }

    /// Invariant: main + async-std → #[async_std::main] attribute.
    #[test]
    fn main_async_std_attribute() {
        assert_eq!(
            executor_attribute(ExecutorChoice::AsyncStd, true),
            Some("#[async_std::main]")
        );
    }

    /// Invariant: main + no executor → None.
    #[test]
    fn main_no_executor_attribute() {
        assert_eq!(executor_attribute(ExecutorChoice::None, true), None);
    }

    /// Invariant: unrelated dependencies don't match.
    #[test]
    fn unrelated_deps_no_executor() {
        let deps = HashMap::from([
            ("serde".to_string(), toml::Value::String("1".into())),
            ("reqwest".to_string(), toml::Value::String("0.11".into())),
        ]);
        assert_eq!(select_executor(&deps), ExecutorChoice::None);
    }
}
