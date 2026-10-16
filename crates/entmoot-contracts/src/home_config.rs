//! Operator home and per-workspace configuration (`~/.entmoot/**/config.json`).
//!
//! Process `ENTMOOT_*` environment variables still win when set. Empty config
//! slots mean unset and continue discovery.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Wire schema id for [`EntmootConfig`].
pub const ENTMOOT_CONFIG_SCHEMA: &str = "entmoot.config.v1";

/// File name under `~/.entmoot/` and each workspace namespace.
pub const ENTMOOT_CONFIG_FILE: &str = "config.json";

/// Subdirectory under a workspace namespace used when `volume` is unset.
pub const ENTMOOT_DEFAULT_STORE: &str = "store";

/// Supported `ENTMOOT_*` keys exposed as empty slots in config for overrides.
pub const ENTMOOT_CONFIG_ENV_KEYS: &[&str] = &[
    "ENTMOOT_DATA_DIR",
    "ENTMOOT_ADAPTERS_ROOT",
    "ENTMOOT_GO_ADAPTER",
    "ENTMOOT_ROSLYN_ADAPTER",
    "ENTMOOT_JAVA_ADAPTER",
    "ENTMOOT_CLANG_ADAPTER",
    "ENTMOOT_PYRIGHT_ADAPTER",
    "ENTMOOT_TYPESCRIPT_ADAPTER",
    "ENTMOOT_JAVA",
    "ENTMOOT_NODE",
    "ENTMOOT_DOTNET",
    "ENTMOOT_PYTHON_EXECUTABLE",
    "ENTMOOT_PYTHON_ENVIRONMENT",
    "ENTMOOT_QUERY_BACKEND",
    "ENTMOOT_QUERY_TOKEN",
];

/// Local operator configuration: volume pointer plus per-key env override slots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntmootConfig {
    /// Schema id for this config document.
    pub schema: String,
    /// `"local"` today; remote transport is rejected until implemented.
    pub mode: String,
    /// Absolute storage volume for this workspace. Empty/None → sibling `store/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    /// Per-key overrides. Empty string means unset (continue discovery).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl EntmootConfig {
    /// Empty local config with every known `ENTMOOT_*` slot present and blank.
    #[must_use]
    pub fn empty_local() -> Self {
        let mut env = BTreeMap::new();
        for key in ENTMOOT_CONFIG_ENV_KEYS {
            env.insert((*key).to_owned(), String::new());
        }
        Self {
            schema: ENTMOOT_CONFIG_SCHEMA.into(),
            mode: "local".into(),
            volume: None,
            env,
        }
    }

    /// Non-empty slot value for `key`, if configured.
    #[must_use]
    pub fn env_slot(&self, key: &str) -> Option<&str> {
        self.env
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// True when mode claims a remote transport (unsupported).
    #[must_use]
    pub fn is_remote(&self) -> bool {
        !self.mode.is_empty() && self.mode != "local"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_local_has_blank_slots_and_round_trips() {
        let config = EntmootConfig::empty_local();
        assert_eq!(config.schema, ENTMOOT_CONFIG_SCHEMA);
        assert_eq!(config.mode, "local");
        assert!(config.volume.is_none());
        assert_eq!(config.env.len(), ENTMOOT_CONFIG_ENV_KEYS.len());
        assert!(config.env_slot("ENTMOOT_GO_ADAPTER").is_none());
        let value = serde_json::to_value(&config).unwrap();
        let restored: EntmootConfig = serde_json::from_value(value).unwrap();
        assert_eq!(restored, config);
    }
}
