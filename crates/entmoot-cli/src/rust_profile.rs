//! Shared Cargo selection for direct production and automatic enrichment.

use std::path::PathBuf;

use anyhow::{bail, Result};
use entmoot_contracts::{RustAnalysisMode, RustBuildProfile, RustFeatureSelection};

#[derive(Debug, Default, clap::Args)]
pub struct RustProfileArgs {
    /// Comma-separated Cargo features, or "all".
    #[arg(long)]
    cargo_features: Option<String>,
    #[arg(long)]
    no_default_features: bool,
    #[arg(long)]
    cargo_target: Option<String>,
    /// Installed sysroot (absolute UTF-8 path); otherwise discover the toolchain.
    #[arg(long)]
    sysroot: Option<PathBuf>,
    /// Compile one repository-relative Cargo crate root as a test target.
    #[arg(long)]
    rust_test_root: Option<String>,
}

impl RustProfileArgs {
    pub fn into_profile(self) -> Result<RustBuildProfile> {
        let features = if self.cargo_features.as_deref() == Some("all") {
            if self.no_default_features {
                bail!("--cargo-features all cannot be combined with --no-default-features");
            }
            RustFeatureSelection::All {}
        } else {
            RustFeatureSelection::Selected {
                names: self
                    .cargo_features
                    .into_iter()
                    .flat_map(|names| {
                        names
                            .split(',')
                            .map(|name| name.trim().to_owned())
                            .collect::<Vec<_>>()
                    })
                    .collect(),
                no_default_features: self.no_default_features,
            }
        };
        let profile = RustBuildProfile {
            features,
            target_triple: self.cargo_target,
            sysroot: self.sysroot,
            analysis: self
                .rust_test_root
                .map_or(RustAnalysisMode::Workspace {}, |crate_root| {
                    RustAnalysisMode::Test { crate_root }
                }),
        };
        profile.validate().map_err(anyhow::Error::msg)?;
        Ok(profile)
    }
}
