//! Workspace-owned adapter readiness profile (`entmoot.adapter_profile.v1`).
//!
//! Written/refreshed by `init` from host discovery. Never authorizes downloads.
//! Process `ENTMOOT_*` overrides remain exclusive when set.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Wire schema id for [`AdapterProfile`].
pub const ADAPTER_PROFILE_SCHEMA: &str = "entmoot.adapter_profile.v1";

/// Relative path under a workspace `.entmoot/` directory.
pub const ADAPTER_PROFILE_FILE: &str = "adapter-profile.json";

/// Obsolete filename from the pre-release native_profile layout (refuse; do not alias).
pub const OBSOLETE_NATIVE_PROFILE_FILE: &str = "native-profile.json";

/// How an adapter or toolchain path was selected for the profile row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdapterProfileSelection {
    /// Exclusive process environment variable was set.
    ProcessEnv,
    /// Path recorded in this profile file.
    Profile,
    /// Catalog relative under adapters root / repo default.
    Catalog,
    /// In-process Cargo feature (rust-analyzer / oxc).
    ProducerFeature,
    /// No path or feature established.
    Unavailable,
}

/// Availability of one adapter family for bind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdapterFamilyAvailability {
    Available,
    Missing,
    Invalid,
    /// Present but not required for the current inventory language set.
    Informational,
}

/// Integrity of a recorded adapter artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdapterIntegrity {
    Ok,
    Mismatch,
    Missing,
    /// Digest was not recorded (file existence only).
    Unchecked,
    Skipped,
}

/// One family row in the workspace adapter profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterProfileFamily {
    /// Catalog / bind family id (`go`, `oxc`, `rust`, …).
    pub family: String,
    pub availability: AdapterFamilyAvailability,
    pub selection: AdapterProfileSelection,
    /// Absolute or workspace-stable adapter path when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Floor / adapter pin label when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    /// Observed host tool version when a probe succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_version: Option<String>,
    /// BLAKE3 of adapter bytes when a build-receipt digest was available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    pub integrity: AdapterIntegrity,
    /// Operator-facing detail (missing path, version mismatch, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Hard gap for bind when this family is required by inventory.
    #[serde(default)]
    pub hard: bool,
}

/// Entmoot-owned adapter configuration for one workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterProfile {
    pub schema_version: String,
    /// Repository-relative or absolute adapters root when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapters_root: Option<String>,
    /// Whether the running CLI was built with in-process producer features.
    #[serde(default)]
    pub producer_features: bool,
    /// Families discovered or expected for this workspace.
    pub families: Vec<AdapterProfileFamily>,
    /// Explicit interpretation constraints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}

impl AdapterProfile {
    /// Empty profile with the current schema version.
    #[must_use]
    pub fn empty(producer_features: bool) -> Self {
        Self {
            schema_version: ADAPTER_PROFILE_SCHEMA.into(),
            adapters_root: None,
            producer_features,
            families: Vec::new(),
            limitations: vec![
                "adapter_profile_records_discovery_only".into(),
                "entmoot_does_not_download_toolchains".into(),
                "process_ENTMOOT_env_overrides_profile_when_set".into(),
            ],
        }
    }

    /// True when every `hard` family row is available.
    #[must_use]
    pub fn bind_ready(&self) -> bool {
        self.families
            .iter()
            .filter(|row| row.hard)
            .all(|row| row.availability == AdapterFamilyAvailability::Available)
    }

    /// Hard gap family ids.
    #[must_use]
    pub fn hard_gaps(&self) -> Vec<&str> {
        self.families
            .iter()
            .filter(|row| row.hard && row.availability != AdapterFamilyAvailability::Available)
            .map(|row| row.family.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_profile_round_trips_and_rejects_unknown_fields() {
        let profile = AdapterProfile {
            schema_version: ADAPTER_PROFILE_SCHEMA.into(),
            adapters_root: Some(".tmp/adapter-build/resolver-adapters".into()),
            producer_features: true,
            families: vec![AdapterProfileFamily {
                family: "go".into(),
                availability: AdapterFamilyAvailability::Missing,
                selection: AdapterProfileSelection::Unavailable,
                path: None,
                pin: Some("go_types_sidecar.isolated_modules.v1".into()),
                observed_version: None,
                digest: None,
                integrity: AdapterIntegrity::Missing,
                detail: Some("adapter not found".into()),
                hard: true,
            }],
            limitations: vec!["adapter_profile_records_discovery_only".into()],
        };
        let value = serde_json::to_value(&profile).unwrap();
        let restored: AdapterProfile = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored, profile);
        assert!(!restored.bind_ready());
        assert_eq!(restored.hard_gaps(), vec!["go"]);

        let mut unknown = value;
        unknown
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::json!(1));
        assert!(serde_json::from_value::<AdapterProfile>(unknown).is_err());
    }

    #[test]
    fn schema_version_is_stable_v1() {
        assert_eq!(ADAPTER_PROFILE_SCHEMA, "entmoot.adapter_profile.v1");
        assert_eq!(ADAPTER_PROFILE_FILE, "adapter-profile.json");
    }
}
