//! Producer-stated package API facts carried in immutable resolver artifacts.
//!
//! Export closure is an explicit producer assertion. Boundary equality is
//! relative to a named policy, never a language-wide compatibility theorem.

use crate::{Address, ProviderSymbolSpelling, ScopeCompleteness};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PackageApiEcosystem {
    Rust,
    Javascript,
    Go,
    Python,
    Java,
    Csharp,
    /// C and C++ packaging; symbol schemes retain language/linkage identity.
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageApiPackage {
    pub ecosystem: PackageApiEcosystem,
    /// Native manifest/module identity. Version strings never establish fit.
    pub name: String,
}

impl PackageApiPackage {
    /// Decode an attributed package coordinate without using its version as
    /// evidence of compatibility. Unsupported ecosystems remain unassessed.
    pub fn from_purl(value: &str) -> Result<Option<Self>, PackageApiError> {
        use std::str::FromStr;
        let package = packageurl::PackageUrl::from_str(value)
            .map_err(|error| PackageApiError(error.to_string()))?;
        let ecosystem = match package.ty() {
            "cargo" => PackageApiEcosystem::Rust,
            "npm" => PackageApiEcosystem::Javascript,
            "golang" => PackageApiEcosystem::Go,
            "pypi" => PackageApiEcosystem::Python,
            "maven" => PackageApiEcosystem::Java,
            "nuget" => PackageApiEcosystem::Csharp,
            "conan" | "generic" => PackageApiEcosystem::Native,
            _ => return Ok(None),
        };
        let mut name = package.namespace().map_or_else(
            || package.name().to_owned(),
            |namespace| format!("{namespace}/{}", package.name()),
        );
        if ecosystem == PackageApiEcosystem::Go {
            if let Some(original) = package.qualifiers().get("go_module_path") {
                let rebuilt = crate::purl_golang_module(original, None)
                    .map_err(|error| PackageApiError(error.to_string()))?;
                let rebuilt = packageurl::PackageUrl::from_str(&rebuilt)
                    .map_err(|error| PackageApiError(error.to_string()))?;
                if rebuilt.namespace() != package.namespace()
                    || rebuilt.name() != package.name()
                    || rebuilt.qualifiers().get("go_module_path") != Some(original)
                {
                    return Err(PackageApiError(
                        "invalid go_module_path coordinate qualifier".into(),
                    ));
                }
                name = original.to_string();
            }
        } else if ecosystem == PackageApiEcosystem::Native {
            name = format!("{}:{name}", package.ty());
        } else if ecosystem == PackageApiEcosystem::Csharp {
            // NuGet lookup identity is case-insensitive. Keep this package API
            // join normalization separate from the preserved provider PURL.
            name.make_ascii_lowercase();
        }
        Ok(Some(Self { ecosystem, name }))
    }

    /// Versionless package coordinate PURL for package-level `depends_on` edges.
    /// Not install identity and not a fit claim.
    pub fn coordinate_purl(&self) -> Result<String, PackageApiError> {
        let map = |error: crate::ProviderCallableRefError| PackageApiError(error.to_string());
        match self.ecosystem {
            PackageApiEcosystem::Rust => crate::purl_cargo(&self.name, None).map_err(map),
            PackageApiEcosystem::Javascript => crate::purl_npm(&self.name, None).map_err(map),
            PackageApiEcosystem::Go => crate::purl_golang_module(&self.name, None).map_err(map),
            PackageApiEcosystem::Python => crate::purl_pypi(&self.name, None).map_err(map),
            PackageApiEcosystem::Java => {
                let (group, artifact) = match self.name.rsplit_once('/') {
                    Some((group, artifact)) if !group.is_empty() && !artifact.is_empty() => {
                        (group, artifact)
                    }
                    _ => {
                        return Err(PackageApiError(
                            "java package coordinate requires group/artifact".into(),
                        ))
                    }
                };
                crate::purl_maven(group, artifact, None).map_err(map)
            }
            PackageApiEcosystem::Csharp => crate::purl_nuget(&self.name, None).map_err(map),
            PackageApiEcosystem::Native => {
                let (ty, rest) = self
                    .name
                    .split_once(':')
                    .unwrap_or(("generic", self.name.as_str()));
                let (namespace, name) = match rest.rsplit_once('/') {
                    Some((namespace, name)) => (Some(namespace), name),
                    None => (None, rest),
                };
                let mut package = packageurl::PackageUrl::new(ty, name)
                    .map_err(|error| PackageApiError(error.to_string()))?;
                if let Some(namespace) = namespace {
                    package
                        .with_namespace(namespace)
                        .map_err(|error| PackageApiError(error.to_string()))?;
                }
                Ok(package.to_string())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageApiTarget {
    pub package: PackageApiPackage,
    /// Exact native export path, including namespace/module qualification.
    pub symbol: ProviderSymbolSpelling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageApiExport {
    pub target: PackageApiTarget,
    /// Inventoried provider declaration, validated before attachment.
    pub declaration: Address,
    pub boundary_policy: String,
    /// Producer's boundary contract digest, distinct from a body hash.
    pub boundary_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageApiSurface {
    pub package: PackageApiPackage,
    /// Symbol/policy universes enumerated by this surface. Absence requires an
    /// exact match here, including when the surface contains no exports.
    pub export_universes: Vec<PackageApiExportUniverse>,
    /// Completeness of export enumeration under the artifact's build context,
    /// including aliases and visibility. Required to conclude export absence;
    /// a present export's explicit boundary remains usable with partial enumeration.
    pub completeness: ScopeCompleteness,
    pub exports: Vec<PackageApiExport>,
    pub limitations: Vec<CompatibilityLimitation>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageApiExportUniverse {
    pub symbol_scheme: String,
    pub boundary_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageApiReference {
    /// Exact selected lock entry and placement when asserted. Absence is a
    /// strengthen-available gap for install identity—not a blocker for seeking,
    /// stitch/`bound`, or assessed boundary `fits`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_instance: Option<LockedPackageInstance>,
    /// Exact resolver observation and alternative supporting this dependency.
    pub observation_id: String,
    pub target_index: usize,
    pub target: PackageApiTarget,
    pub boundary_policy: String,
    /// Contract observed in the dependency against which the consumer bound.
    pub expected_boundary: Option<String>,
}

/// Producer/operator assertion of a selected dependency under the resolver snapshot/build.
/// The producer owns lock interpretation and retains its inputs and selection policy.
/// Entmoot validates identity and cited bytes; it does not interpret lock entries or
/// authenticate an installation. A digest is not a retained copy, and a version or
/// package name alone cannot identify an installed instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LockedPackageInstance {
    pub instance_id: String,
    pub purl: String,
    /// Repository-relative path of the consumer's lock input.
    pub lock_path: String,
    pub lock_digest: crate::ContentDigest,
    /// Opaque entry locator interpreted only by the emitting producer.
    pub entry: String,
    /// Consumer-relative installation slot; aliases/duplicate copies stay distinct.
    pub placement: String,
}

impl LockedPackageInstance {
    pub fn derive_id(&self) -> String {
        crate::hash_parts(&[
            "entmoot.lock_instance.v1",
            &self.purl,
            &self.lock_path,
            self.lock_digest.as_str(),
            &self.entry,
            &self.placement,
        ])
    }

    pub fn validate(&self) -> Result<(), PackageApiError> {
        use std::str::FromStr;
        let purl = packageurl::PackageUrl::from_str(&self.purl)
            .map_err(|e| PackageApiError(e.to_string()))?;
        if purl.version().is_none_or(str::is_empty)
            || purl.to_string() != self.purl
            || self.entry.trim().is_empty()
            || self.entry.len() > 4096
        {
            return Err(PackageApiError(
                "lock instance requires canonical versioned PURL and exact entry".into(),
            ));
        }
        crate::ContentDigest::parse(self.lock_digest.as_str())
            .map_err(|e| PackageApiError(e.to_string()))?;
        for path in [&self.lock_path, &self.placement] {
            if crate::normalize_repo_path(path).map_err(|e| PackageApiError(e.to_string()))?
                != *path
            {
                return Err(PackageApiError(
                    "lock path and placement must be canonical".into(),
                ));
            }
        }
        if self.instance_id != self.derive_id() {
            return Err(PackageApiError("lock instance identity mismatch".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageApiEvidence {
    pub surfaces: Vec<PackageApiSurface>,
    pub references: Vec<PackageApiReference>,
    /// Explicit coverage of direct dependency sites in the artifact's scope.
    pub references_complete: bool,
    pub limitations: Vec<CompatibilityLimitation>,
}

/// Closed limitation codes for package-API evidence and compatibility receipts.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityLimitationCode {
    /// Producer-stated free-form residual retained as attributable detail.
    ProducerStated,
    /// Attached resolver artifact bytes were unavailable from the object store.
    ArtifactUnavailable,
    /// Attached resolver artifact did not emit package_api evidence.
    PackageApiNotProduced,
    /// Consumer publication has no package_api evidence for stitch.
    PackageApiEvidenceMissing,
    /// Dependency sites were present but lacked a stitchable package binding.
    UnqualifiedDependencySites,
    /// Consumer dependency scope is not complete relative to its universe.
    ConsumerDependencyScopeIncomplete,
    /// No consumer package sites were evaluable under the request.
    NoEvaluablePackageSites,
    /// Consumer has zero importer package_api references for a requested provider
    /// coordinate (`detail` carries `consumer_missing_importer_refs:<coord>`).
    ConsumerMissingImporterRefs,
    /// Boundary equality is under the producer policy only (not SemVer;
    /// typecheck success is not today’s fit oracle).
    BoundaryEqualityPolicyOnly,
    /// Selected lock/install identity was not asserted; `bound`/`fits` may still hold.
    LockInstanceUnasserted,
    /// A dependency package-root mapping was asserted but the root was missing.
    MappedPackageRootMissing,
}

/// Typed limitation envelope for package-API evidence and compatibility receipts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityLimitation {
    pub code: CompatibilityLimitationCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

impl CompatibilityLimitation {
    /// Retain a producer-stated residual as attributable detail under a closed code.
    pub fn producer_stated(detail: impl Into<String>) -> Self {
        Self {
            code: CompatibilityLimitationCode::ProducerStated,
            artifact_id: None,
            publication: None,
            detail: Some(detail.into()),
            count: None,
        }
    }

    pub fn coded(code: CompatibilityLimitationCode) -> Self {
        Self {
            code,
            artifact_id: None,
            publication: None,
            detail: None,
            count: None,
        }
    }

    pub fn with_artifact(mut self, artifact_id: impl Into<String>) -> Self {
        self.artifact_id = Some(artifact_id.into());
        self
    }

    pub fn with_publication(mut self, publication: impl Into<String>) -> Self {
        self.publication = Some(publication.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_count(mut self, count: u64) -> Self {
        self.count = Some(count);
        self
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid package API evidence: {0}")]
pub struct PackageApiError(pub String);

impl PackageApiEvidence {
    pub fn validate(&self, artifact: &crate::ResolverArtifact) -> Result<(), PackageApiError> {
        let invalid = |message: &str| PackageApiError(message.into());
        let ecosystem = match artifact.language.as_str() {
            "rust" => PackageApiEcosystem::Rust,
            "javascript" | "typescript" | "tsx" => PackageApiEcosystem::Javascript,
            "go" => PackageApiEcosystem::Go,
            "python" => PackageApiEcosystem::Python,
            "java" => PackageApiEcosystem::Java,
            "csharp" => PackageApiEcosystem::Csharp,
            "c" | "cpp" | "cxx" => PackageApiEcosystem::Native,
            _ => return Err(invalid("unsupported package API ecosystem")),
        };
        let digest = |value: &str| {
            crate::ContentDigest::parse(value.to_owned())
                .map(|_| ())
                .map_err(|error| PackageApiError(error.to_string()))
        };
        let target = |value: &PackageApiTarget| {
            if value.package.ecosystem != ecosystem
                || value.package.name.trim().is_empty()
                || value.symbol.scheme.trim().is_empty()
                || value.symbol.spelling.trim().is_empty()
            {
                Err(invalid("package and qualified symbol must be nonempty"))
            } else {
                Ok(())
            }
        };
        let mut packages = std::collections::BTreeSet::new();
        for surface in &self.surfaces {
            if surface.package.ecosystem != ecosystem
                || surface.package.name.trim().is_empty()
                || !packages.insert(&surface.package)
            {
                return Err(invalid("empty or duplicate package surface"));
            }
            let mut universes = std::collections::BTreeSet::new();
            for universe in &surface.export_universes {
                if universe.symbol_scheme.trim().is_empty()
                    || universe.boundary_policy.trim().is_empty()
                    || !universes.insert(universe)
                {
                    return Err(invalid("empty or duplicate export universe"));
                }
            }
            if surface.completeness == ScopeCompleteness::CompleteRelativeToUniverse
                && universes.is_empty()
            {
                return Err(invalid(
                    "complete surface requires an explicit export universe",
                ));
            }
            let mut symbols = std::collections::BTreeSet::new();
            for export in &surface.exports {
                target(&export.target)?;
                if export.target.package != surface.package
                    || export.boundary_policy.trim().is_empty()
                    || !symbols
                        .insert((&export.target.symbol.scheme, &export.target.symbol.spelling))
                {
                    return Err(invalid(
                        "export has a wrong package, empty policy, or duplicate symbol",
                    ));
                }
                export
                    .declaration
                    .validate()
                    .map_err(|error| PackageApiError(error.to_string()))?;
                digest(&export.boundary_digest)?;
            }
        }
        let observations: std::collections::BTreeMap<_, _> = artifact
            .observations
            .iter()
            .map(|row| (row.occurrence_id.as_str(), row))
            .collect();
        let mut sites = std::collections::BTreeSet::new();
        for reference in &self.references {
            target(&reference.target)?;
            if let Some(instance) = &reference.lock_instance {
                instance.validate()?;
                if PackageApiPackage::from_purl(&instance.purl)?
                    != Some(reference.target.package.clone())
                {
                    return Err(invalid(
                        "lock instance package disagrees with export target",
                    ));
                }
            }
            if reference.boundary_policy.trim().is_empty()
                || !sites.insert((&reference.observation_id, reference.target_index))
            {
                return Err(invalid("reference has an empty policy or duplicate target"));
            }
            let observed = observations
                .get(reference.observation_id.as_str())
                .and_then(|row| row.targets.get(reference.target_index))
                .ok_or_else(|| invalid("reference has no native binding target"))?;
            if observed.provider_symbol.as_ref() != Some(&reference.target.symbol) {
                return Err(invalid(
                    "reference disagrees with the native symbol binding",
                ));
            }
            if let Some(purl) = observed
                .provider_callable
                .as_ref()
                .and_then(|callable| callable.package.purl.as_deref())
            {
                if PackageApiPackage::from_purl(purl)? != Some(reference.target.package.clone()) {
                    return Err(invalid(
                        "reference package disagrees with native package coordinate",
                    ));
                }
                if let Some(instance) = &reference.lock_instance {
                    use std::str::FromStr;
                    let native = packageurl::PackageUrl::from_str(purl)
                        .map_err(|e| PackageApiError(e.to_string()))?;
                    let locked = packageurl::PackageUrl::from_str(&instance.purl)
                        .map_err(|e| PackageApiError(e.to_string()))?;
                    if native.version().is_some() && native.version() != locked.version() {
                        return Err(invalid(
                            "lock instance version disagrees with native binding",
                        ));
                    }
                }
            }
            if let Some(value) = &reference.expected_boundary {
                digest(value)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_module_qualifier_is_lossless_and_cannot_retarget_the_base() {
        let mixed = crate::purl_golang_module("github.com/XSAM/otelsql", Some("v0.40.0")).unwrap();
        let lower = crate::purl_golang_module("github.com/xsam/otelsql", Some("v0.40.0")).unwrap();
        assert_ne!(mixed, lower);
        assert_eq!(
            crate::purl_address(&mixed).unwrap(),
            crate::Address::Purl {
                purl: mixed.clone()
            }
        );
        assert_eq!(
            PackageApiPackage::from_purl(&mixed).unwrap().unwrap().name,
            "github.com/XSAM/otelsql"
        );
        assert_eq!(
            PackageApiPackage::from_purl(&lower).unwrap().unwrap().name,
            "github.com/xsam/otelsql"
        );
        assert!(PackageApiPackage::from_purl(
            "pkg:golang/github.com/other/otelsql?go_module_path=github.com%2FXSAM%2Fotelsql"
        )
        .is_err());
        assert!(PackageApiPackage::from_purl(
            "pkg:golang/github.com/xsam/otelsql?go_module_path=github.com%2Fxsam%2Fotelsql"
        )
        .is_err());
    }

    #[test]
    fn coordinates_cover_native_producer_ecosystems_without_cross_registry_collisions() {
        for (purl, ecosystem, name) in [
            (
                "pkg:maven/org.example/api@1.0",
                PackageApiEcosystem::Java,
                "org.example/api",
            ),
            (
                "pkg:nuget/Example.Api@1.0",
                PackageApiEcosystem::Csharp,
                "example.api",
            ),
            (
                "pkg:conan/zlib@1.3",
                PackageApiEcosystem::Native,
                "conan:zlib",
            ),
            (
                "pkg:generic/zlib@1.3",
                PackageApiEcosystem::Native,
                "generic:zlib",
            ),
        ] {
            let package = PackageApiPackage::from_purl(purl).unwrap().unwrap();
            assert_eq!(package.ecosystem, ecosystem);
            assert_eq!(package.name, name);
            let coordinate = package.coordinate_purl().unwrap();
            let roundtrip = PackageApiPackage::from_purl(&coordinate).unwrap().unwrap();
            assert_eq!(roundtrip.ecosystem, ecosystem);
            assert_eq!(roundtrip.name, name);
        }
    }

    #[test]
    fn pypi_coordinates_are_a_supported_package_boundary() {
        let purl = crate::purl_pypi("Example_Package", Some("1.2.3")).unwrap();
        let package = PackageApiPackage::from_purl(&purl).unwrap().unwrap();
        assert_eq!(package.ecosystem, PackageApiEcosystem::Python);
        assert_eq!(package.name, "example-package");
        assert_eq!(
            PackageApiPackage::from_purl("pkg:nuget/Example")
                .unwrap()
                .unwrap()
                .ecosystem,
            PackageApiEcosystem::Csharp
        );
    }
}
