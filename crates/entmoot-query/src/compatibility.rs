//! Read-only package boundary projection over independent immutable publications.
//! Native artifacts remain the source of evidence; graphs are never merged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use entmoot_contracts::package_api::{
    CompatibilityLimitation, CompatibilityLimitationCode, PackageApiEcosystem, PackageApiExport,
    PackageApiPackage, PackageApiReference, PackageApiSurface,
};
use entmoot_contracts::query::{
    BoundaryKind, CompatibilityBoundary, CompatibilityPublication, CompatibilityReason,
    CompatibilityRequest, CompatibilityResult, CompatibilitySite, CompatibilityVerdict as Verdict,
    QueryServiceError, PACKAGE_COMPATIBILITY_SCHEMA,
};
use entmoot_contracts::{
    address_identity_key, topology_node_address, Address, ArtifactFormat, ArtifactManifest,
    ContentDigest, EvidenceRef, ScopeCompleteness,
};

use crate::{FilesystemArtifactObjectRepository, NodeKind, TopologyGraph};

fn invalid(error: impl std::fmt::Display) -> QueryServiceError {
    QueryServiceError::InvalidRequest(error.to_string())
}

/// Resolve a local graph, publication pointer, or published checkout. Explicit
/// pointers preserve custom object-store locations; portable requests use digests.
pub fn publication_input(
    path: &Path,
) -> Result<(PathBuf, Option<PathBuf>, Option<PathBuf>), QueryServiceError> {
    let path = if path.is_dir() {
        let pointer = entmoot_store::location::storage_directory(path)
            .map_err(invalid)?
            .join("current.json");
        if pointer.is_file() {
            pointer
        } else {
            path.join("current.json")
        }
    } else {
        path.to_owned()
    };
    if path.file_name().is_some_and(|name| name == "current.json") {
        let publication = entmoot_store::location::read_publication(&path).map_err(invalid)?;
        Ok((
            publication.current.graph,
            Some(publication.current.store),
            Some(publication.current.source_root),
        ))
    } else {
        Ok((path, None, None))
    }
}

/// Shared CLI/MCP provider loading. Discovery is bounded and has no effects.
pub fn load_providers(
    root: &Path,
    explicit: &[PathBuf],
) -> Result<Vec<PackagePublication>, QueryServiceError> {
    let paths = if explicit.is_empty() {
        discover_dependencies(root)?
    } else {
        explicit
            .iter()
            .map(|path| publication_input(path).map(|(graph, store, _)| (graph, store)))
            .collect::<Result<_, _>>()?
    };
    paths
        .into_iter()
        .map(|(path, store)| PackagePublication::load(&path, store.as_deref()))
        .collect()
}

/// Discover bounded, local dependency graphs without following directory links.
/// Pointer targets must remain inside their dependency directory.
pub fn discover_dependencies(
    root: &Path,
) -> Result<Vec<(PathBuf, Option<PathBuf>)>, QueryServiceError> {
    let deps = entmoot_store::location::storage_directory(root)
        .map_err(invalid)?
        .join("deps");
    if !deps.exists() {
        return Ok(Vec::new());
    }
    let canonical_root = entmoot_store::location::storage_directory(root)
        .map_err(invalid)?
        .canonicalize()
        .map_err(invalid)?;
    let canonical_deps = deps.canonicalize().map_err(invalid)?;
    if !canonical_deps.starts_with(&canonical_root) {
        return Err(invalid("dependency discovery escapes workspace"));
    }
    let mut result = Vec::new();
    for (count, entry) in std::fs::read_dir(&deps).map_err(invalid)?.enumerate() {
        if count >= 256 {
            return Err(invalid(
                "dependency discovery exceeds 256 entries; configure explicit providers",
            ));
        }
        let entry = entry.map_err(invalid)?;
        if !entry.file_type().map_err(invalid)?.is_dir() {
            continue;
        }
        let directory = entry.path().canonicalize().map_err(invalid)?;
        if !directory.starts_with(&canonical_deps) {
            return Err(invalid("dependency directory escapes discovery root"));
        }
        let namespace = entmoot_store::location::storage_directory(&directory).map_err(invalid)?;
        let pointer = if namespace.join("current.json").is_file() {
            namespace.join("current.json")
        } else {
            directory.join("current.json")
        };
        let (graph, store, allowed) = if pointer.is_file() {
            let publication =
                entmoot_store::location::read_publication(&pointer).map_err(invalid)?;
            if publication
                .current
                .source_root
                .canonicalize()
                .map_err(invalid)?
                != directory
            {
                return Err(invalid(
                    "dependency publication belongs to a different source root",
                ));
            }
            let allowed = if pointer.starts_with(&namespace) {
                namespace.canonicalize().map_err(invalid)?
            } else {
                directory.clone()
            };
            (
                publication.current.graph,
                Some(publication.current.store),
                allowed,
            )
        } else {
            (directory.join("logic.graph.v1"), None, directory.clone())
        };
        if !graph.is_file() {
            continue;
        }
        let graph = graph.canonicalize().map_err(invalid)?;
        if !graph.starts_with(&allowed) {
            return Err(invalid("dependency graph escapes its directory"));
        }
        let store = store
            .map(|path| path.canonicalize().map_err(invalid))
            .transpose()?;
        if store
            .as_ref()
            .is_some_and(|path| !path.starts_with(&allowed))
        {
            return Err(invalid("dependency store escapes its directory"));
        }
        result.push((graph, store));
    }
    result.sort();
    Ok(result)
}

/// Validate exported declarations against the same inventory and scope used at attach.

#[derive(Debug, Clone)]
struct Surface {
    value: PackageApiSurface,
    artifact_id: String,
    pointer: String,
    complete_scope: bool,
    export_index: BTreeMap<(String, String), usize>,
}

#[derive(Debug, Clone)]
struct Reference {
    value: PackageApiReference,
    location: Address,
    evidence: Vec<EvidenceRef>,
    ambiguous: bool,
    // Scoped to the artifact that asserted this reference, never provider warnings.
    mapped_root_missing: bool,
}

/// Small evidence projection. No graph nodes or source buffers are retained.
#[derive(Debug, Clone)]
pub struct PackagePublication {
    receipt: CompatibilityPublication,
    surfaces: Vec<Surface>,
    references: Vec<Reference>,
    references_complete: bool,
    limitations: Vec<entmoot_contracts::package_api::CompatibilityLimitation>,
}

impl PackagePublication {
    /// Read a pinned publication or explicit graph export and verified artifacts.
    /// A missing artifact is an explicit gap; corrupted evidence is a hard error.
    pub fn load(path: &Path, store: Option<&Path>) -> Result<Self, QueryServiceError> {
        let (graph, digest) = if entmoot_store::KnowledgeViewLocator::read(path)
            .map_err(invalid)?
            .is_some()
        {
            use entmoot_store::read::RepositoryReader;
            let reader = entmoot_store::read::SqliteReader::new(path);
            let view = reader.open_read().map_err(invalid)?;
            let digest = view
                .inventory_export_digest()
                .map_err(invalid)?
                .ok_or_else(|| invalid("compatibility publication commitment is missing"))?;
            let graph = view
                .load_inventory(false)
                .map_err(invalid)?
                .ok_or_else(|| invalid("compatibility inventory is missing"))?;
            (graph, digest)
        } else {
            let bytes = std::fs::read(path).map_err(invalid)?;
            let mut graph: TopologyGraph = serde_json::from_slice(&bytes).map_err(invalid)?;
            graph.rebuild_indexes();
            (graph, entmoot_contracts::blake3_hex(&bytes))
        };
        let store = store
            .map(Path::to_path_buf)
            .unwrap_or_else(|| entmoot_store::location::default_store_path(path));
        let repository = FilesystemArtifactObjectRepository::new(
            entmoot_store::location::artifact_store_path(&store).map_err(invalid)?,
        );
        Self::from_graph(&graph, digest, |manifest| {
            let digest = ContentDigest::parse(manifest.content_digest.clone()).map_err(invalid)?;
            match repository.get(&digest) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(crate::ArtifactStoreError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    Ok(None)
                }
                Err(error) => Err(invalid(error)),
            }
        })
    }

    /// Project validated attached artifacts supplied by the composition root.
    pub fn from_graph(
        graph: &TopologyGraph,
        publication: String,
        read: impl FnMut(&ArtifactManifest) -> Result<Option<Vec<u8>>, QueryServiceError>,
    ) -> Result<Self, QueryServiceError> {
        graph.validate().map_err(invalid)?;
        Self::from_validated_graph(graph, publication, read)
    }

    pub fn from_validated_graph(
        graph: &TopologyGraph,
        publication: String,
        mut read: impl FnMut(&ArtifactManifest) -> Result<Option<Vec<u8>>, QueryServiceError>,
    ) -> Result<Self, QueryServiceError> {
        ContentDigest::parse(publication.clone()).map_err(invalid)?;
        let mut projection = Self {
            receipt: CompatibilityPublication {
                publication,
                snapshot: graph.snapshot.clone(),
            },
            surfaces: Vec::new(),
            references: Vec::new(),
            references_complete: true,
            limitations: Vec::new(),
        };
        let mut source_locations = BTreeMap::new();
        for node in graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::CallStub)
        {
            let path = node
                .path
                .clone()
                .ok_or_else(|| invalid("call site path is absent"))?;
            let start_byte = node
                .start_byte
                .map(u32::try_from)
                .transpose()
                .map_err(invalid)?;
            let end_byte = node
                .end_byte
                .map(u32::try_from)
                .transpose()
                .map_err(invalid)?;
            let location = Address::RepoPath {
                repository_id: Some(graph.snapshot.repository_id.clone()),
                path: path.clone(),
                start_byte,
                end_byte,
            };
            for address in std::iter::once(topology_node_address(&node.id))
                .chain(node.address_id.as_ref().map(|id| Address::AddressId {
                    address_id: id.clone(),
                }))
                .chain(node.logic_address.as_ref().map(|id| Address::LogicAddress {
                    logic_address: id.clone(),
                }))
                .chain(std::iter::once(location.clone()))
                .chain(std::iter::once(Address::RepoPath {
                    repository_id: None,
                    path,
                    start_byte,
                    end_byte,
                }))
            {
                source_locations.insert(address_identity_key(&address), location.clone());
            }
        }
        let source_location = |address: &Address| {
            source_locations
                .get(&address_identity_key(address))
                .cloned()
                .ok_or_else(|| invalid("native site location is not represented"))
        };
        let mut observed = false;
        for manifest in graph
            .artifacts
            .iter()
            .filter(|row| row.format == ArtifactFormat::Resolver)
        {
            manifest.validate_structure().map_err(invalid)?;
            manifest
                .verify_against_snapshot(&graph.snapshot)
                .map_err(invalid)?;
            let Some(bytes) = read(manifest)? else {
                projection.references_complete = false;
                projection.limitations.push(
                    CompatibilityLimitation::coded(
                        CompatibilityLimitationCode::ArtifactUnavailable,
                    )
                    .with_artifact(manifest.artifact_id.clone()),
                );
                continue;
            };
            manifest.verify_content_digest(&bytes).map_err(invalid)?;
            let artifact = crate::artifact::resolver::parse_scoped_artifact(&bytes, manifest)
                .map_err(invalid)?;
            crate::artifact::resolver::join_inventory_bindings(
                graph,
                manifest,
                &artifact.language,
                &artifact.observations,
            )
            .map_err(invalid)?;
            validate_package_inventory(graph, manifest, artifact.package_api.as_ref())?;
            // The package contract may cover only some native bindings. Retain
            // every other attributed external target as an unassessed site, for
            // all ecosystems, without manufacturing an expected boundary.
            let represented: BTreeSet<_> = artifact
                .package_api
                .as_ref()
                .map(|api| {
                    api.references
                        .iter()
                        .map(|row| (row.observation_id.as_str(), row.target_index))
                        .collect()
                })
                .unwrap_or_default();
            let mut unqualified = BTreeMap::<&str, usize>::new();
            for (observation_index, observation) in artifact.observations.iter().enumerate() {
                for (target_index, target) in observation.targets.iter().enumerate() {
                    if represented.contains(&(observation.occurrence_id.as_str(), target_index))
                        || matches!(target.address, Address::RepoPath { .. })
                        || matches!(
                            observation.outcome,
                            entmoot_contracts::ResolverObservationOutcome::RepositoryCallable
                                | entmoot_contracts::ResolverObservationOutcome::RepositorySymbol
                        )
                    {
                        continue;
                    }
                    let Some(callable) = &target.provider_callable else {
                        *unqualified
                            .entry("external_target_metadata_unavailable")
                            .or_default() += 1;
                        continue;
                    };
                    if callable.package.role == entmoot_contracts::ProviderPackageRole::Stdlib {
                        continue;
                    }
                    let Some(purl) = &callable.package.purl else {
                        *unqualified
                            .entry("external_package_coordinate_unavailable")
                            .or_default() += 1;
                        continue;
                    };
                    let Some(package) = PackageApiPackage::from_purl(purl).map_err(invalid)? else {
                        *unqualified
                            .entry("package_ecosystem_unassessed")
                            .or_default() += 1;
                        continue;
                    };
                    let Some(symbol) = &target.provider_symbol else {
                        *unqualified
                            .entry("external_symbol_unavailable")
                            .or_default() += 1;
                        continue;
                    };
                    projection.references_complete = false;
                    projection.references.push(Reference {
                        mapped_root_missing: artifact
                            .package_api
                            .as_ref()
                            .is_some_and(|api| mapped_root_missing(&api.limitations, &package)),
                        value: PackageApiReference {
                            lock_instance: None,
                            observation_id: observation.occurrence_id.clone(),
                            target_index,
                            target: entmoot_contracts::package_api::PackageApiTarget {
                                package,
                                symbol: symbol.clone(),
                            },
                            boundary_policy: "unassessed".into(),
                            expected_boundary: None,
                        },
                        location: source_location(&observation.source.call_site)?,
                        evidence: vec![EvidenceRef {
                            artifact_id: manifest.artifact_id.clone(),
                            provider_pointer: format!(
                                "/observations/{observation_index}/targets/{target_index}"
                            ),
                            record_id: Some(observation.occurrence_id.clone()),
                        }],
                        ambiguous: observation.targets.len() != 1
                            || matches!(
                                observation.outcome,
                                entmoot_contracts::ResolverObservationOutcome::Ambiguous
                            ),
                    });
                }
            }
            if !unqualified.is_empty() {
                projection.references_complete = false;
                projection
                    .limitations
                    .extend(unqualified.into_iter().map(|(reason, count)| {
                        CompatibilityLimitation::coded(
                            CompatibilityLimitationCode::UnqualifiedDependencySites,
                        )
                        .with_artifact(manifest.artifact_id.clone())
                        .with_detail(reason)
                        .with_count(count as u64)
                    }));
            }
            let Some(api) = &artifact.package_api else {
                projection.references_complete = false;
                projection.limitations.push(
                    CompatibilityLimitation::coded(
                        CompatibilityLimitationCode::PackageApiNotProduced,
                    )
                    .with_artifact(manifest.artifact_id.clone()),
                );
                continue;
            };
            observed = true;
            let complete_scope = manifest.scope.completeness
                == ScopeCompleteness::CompleteRelativeToUniverse
                && manifest.scope.producer_skips.is_empty();
            projection.references_complete &= api.references_complete && complete_scope;
            projection
                .limitations
                .extend(api.limitations.iter().cloned().map(|mut gap| {
                    gap.artifact_id = Some(manifest.artifact_id.clone());
                    gap
                }));
            for (index, value) in api.surfaces.iter().enumerate() {
                projection.surfaces.push(Surface {
                    value: value.clone(),
                    artifact_id: manifest.artifact_id.clone(),
                    pointer: format!("/package_api/surfaces/{index}"),
                    complete_scope,
                    export_index: value
                        .exports
                        .iter()
                        .enumerate()
                        .map(|(index, export)| {
                            (
                                (
                                    export.target.symbol.scheme.clone(),
                                    export.target.symbol.spelling.clone(),
                                ),
                                index,
                            )
                        })
                        .collect(),
                });
            }
            let observations: BTreeMap<_, _> = artifact
                .observations
                .iter()
                .enumerate()
                .map(|(index, row)| (row.occurrence_id.as_str(), (index, row)))
                .collect();
            for (index, value) in api.references.iter().enumerate() {
                let (observation_index, observation) = observations
                    .get(value.observation_id.as_str())
                    .ok_or_else(|| invalid("package reference observation missing"))?;
                let evidence = |pointer: String| EvidenceRef {
                    artifact_id: manifest.artifact_id.clone(),
                    provider_pointer: pointer,
                    record_id: Some(value.observation_id.clone()),
                };
                projection.references.push(Reference {
                    mapped_root_missing: mapped_root_missing(
                        &api.limitations,
                        &value.target.package,
                    ),
                    value: value.clone(),
                    location: source_location(&observation.source.call_site)?,
                    ambiguous: observation.targets.len() != 1
                        || matches!(
                            observation.outcome,
                            entmoot_contracts::ResolverObservationOutcome::Ambiguous
                        ),
                    evidence: vec![
                        evidence(format!("/package_api/references/{index}")),
                        evidence(format!(
                            "/observations/{observation_index}/targets/{}",
                            value.target_index
                        )),
                    ],
                });
            }
        }
        projection.references_complete &= observed;
        if !observed {
            projection.limitations.push(CompatibilityLimitation::coded(
                CompatibilityLimitationCode::PackageApiEvidenceMissing,
            ));
        }
        Ok(projection)
    }

    pub fn id(&self) -> &str {
        &self.receipt.publication
    }
}

/// Configured read-only catalog; requests select content identities, never paths.
#[derive(Debug, Clone)]
pub struct CompatibilityCatalog {
    consumer: PackagePublication,
    providers: BTreeMap<String, PackagePublication>,
}

impl CompatibilityCatalog {
    pub fn new(
        consumer: PackagePublication,
        providers: Vec<PackagePublication>,
    ) -> Result<Self, QueryServiceError> {
        let mut catalog = Self {
            consumer,
            providers: BTreeMap::new(),
        };
        for provider in providers {
            if catalog
                .providers
                .insert(provider.id().to_owned(), provider)
                .is_some()
            {
                return Err(invalid("duplicate provider publication"));
            }
        }
        Ok(catalog)
    }

    pub fn request(&self) -> CompatibilityRequest {
        CompatibilityRequest {
            consumer_publication: self.consumer.id().into(),
            provider_publications: self.providers.keys().cloned().collect(),
            boundary_kind: BoundaryKind::PackageApi,
        }
    }

    pub fn assess(
        &self,
        request: CompatibilityRequest,
    ) -> Result<CompatibilityResult, QueryServiceError> {
        ContentDigest::parse(request.consumer_publication.clone()).map_err(invalid)?;
        if request.consumer_publication != self.consumer.id() {
            return Err(invalid("consumer publication is not configured"));
        }
        let mut selected = BTreeSet::new();
        let mut providers = Vec::new();
        let mut surfaces: BTreeMap<&PackageApiPackage, Vec<(&PackagePublication, &Surface)>> =
            BTreeMap::new();
        let mut limitations = self.consumer.limitations.clone();
        for id in &request.provider_publications {
            ContentDigest::parse(id.clone()).map_err(invalid)?;
            if !selected.insert(id) {
                return Err(invalid("duplicate provider publication"));
            }
            let provider = self
                .providers
                .get(id)
                .ok_or_else(|| invalid("provider publication is not configured"))?;
            providers.push(provider.receipt.clone());
            limitations.extend(provider.limitations.iter().cloned().map(|mut gap| {
                gap.publication = Some(id.clone());
                gap
            }));
            for surface in &provider.surfaces {
                limitations.extend(surface.value.limitations.iter().cloned().map(|mut gap| {
                    gap.publication = Some(id.clone());
                    gap
                }));
                surfaces
                    .entry(&surface.value.package)
                    .or_default()
                    .push((provider, surface));
            }
        }
        let mut occurrences: BTreeMap<String, usize> = BTreeMap::new();
        for reference in &self.consumer.references {
            *occurrences
                .entry(address_identity_key(&reference.location))
                .or_default() += 1;
        }
        let mut sites = Vec::new();
        for reference in &self.consumer.references {
            let mut site = CompatibilitySite {
                lock_instance: reference.value.lock_instance.clone(),
                location: reference.location.clone(),
                target: reference.value.target.clone(),
                verdict: Verdict::IncompleteBind,
                reason: CompatibilityReason::ProviderExportEvidenceMissing,
                provider_publication: None,
                boundary_policy: reference.value.boundary_policy.clone(),
                expected_boundary: reference.value.expected_boundary.clone(),
                observed_boundary: None,
                evidence: reference.evidence.clone(),
            };
            if reference.ambiguous
                || occurrences
                    .get(&address_identity_key(&reference.location))
                    .copied()
                    .unwrap_or(0)
                    != 1
            {
                site.verdict = Verdict::Unknown;
                site.reason = CompatibilityReason::AmbiguousConsumerBinding;
            } else if reference.value.expected_boundary.is_none()
                || reference.value.boundary_policy == "unassessed"
            {
                match surfaces
                    .get(&reference.value.target.package)
                    .map(Vec::as_slice)
                {
                    Some([(provider, surface)]) => {
                        locate_unassessed_export(&mut site, provider, surface);
                        if site.verdict != Verdict::Bound && reference.mapped_root_missing {
                            site.verdict = Verdict::BrokenAt;
                            site.reason = CompatibilityReason::MappedPackageRootMissing;
                        }
                    }
                    Some([]) | None => {
                        if reference.mapped_root_missing {
                            site.verdict = Verdict::BrokenAt;
                            site.reason = CompatibilityReason::MappedPackageRootMissing;
                        } else {
                            site.reason = CompatibilityReason::ConsumerBoundaryUnassessed;
                        }
                    }
                    Some(_) => {
                        site.verdict = Verdict::Unknown;
                        site.reason = CompatibilityReason::AmbiguousProviderPublications;
                    }
                }
            } else if let Some(candidates) = surfaces.get(&reference.value.target.package) {
                match candidates.as_slice() {
                    [(provider, surface)] => {
                        compare_site(&mut site, provider, surface);
                        if site.verdict == Verdict::Fits && reference.value.lock_instance.is_none()
                        {
                            limitations.push(
                                CompatibilityLimitation::coded(
                                    CompatibilityLimitationCode::LockInstanceUnasserted,
                                )
                                .with_detail(reference.value.target.package.name.clone()),
                            );
                        }
                    }
                    _ => {
                        site.verdict = Verdict::Unknown;
                        site.reason = CompatibilityReason::AmbiguousProviderPublications;
                    }
                }
            } else if reference.mapped_root_missing {
                site.verdict = Verdict::BrokenAt;
                site.reason = CompatibilityReason::MappedPackageRootMissing;
            }
            sites.push(site);
        }
        sites.sort_by_key(|site| {
            (
                site.target.package.clone(),
                site.target.symbol.scheme.clone(),
                site.target.symbol.spelling.clone(),
                address_identity_key(&site.location),
            )
        });
        let mut packages: BTreeMap<PackageApiPackage, Vec<Verdict>> = BTreeMap::new();
        for site in &sites {
            packages
                .entry(site.target.package.clone())
                .or_default()
                .push(site.verdict);
        }
        let boundaries = packages
            .into_iter()
            .map(|(package, verdicts)| CompatibilityBoundary {
                package,
                verdict: aggregate(&verdicts),
                sites: verdicts.len(),
            })
            .collect();
        let mut verdicts: Vec<_> = sites.iter().map(|site| site.verdict).collect();
        if !self.consumer.references_complete {
            limitations.push(CompatibilityLimitation::coded(
                CompatibilityLimitationCode::ConsumerDependencyScopeIncomplete,
            ));
            // Partial dependency scope stays a limitation. Do not force aggregate
            // incomplete_bind when every assessed site already fits or is bound.
            let assessed_all_usable = !sites.is_empty()
                && sites
                    .iter()
                    .all(|site| matches!(site.verdict, Verdict::Fits | Verdict::Bound));
            if !assessed_all_usable {
                verdicts.push(Verdict::IncompleteBind);
            }
        }
        if sites.is_empty() {
            verdicts.push(Verdict::IncompleteBind);
            limitations.push(CompatibilityLimitation::coded(
                CompatibilityLimitationCode::NoEvaluablePackageSites,
            ));
            // Loud preflight: name each requested provider coordinate with zero
            // consumer importer refs so IncompleteBind is not a silent empty bag.
            let mut provider_coords: BTreeSet<String> = BTreeSet::new();
            for provider in self.providers.values() {
                for surface in &provider.surfaces {
                    provider_coords.insert(format!(
                        "{}:{}",
                        match surface.value.package.ecosystem {
                            PackageApiEcosystem::Python => "python",
                            PackageApiEcosystem::Javascript => "javascript",
                            PackageApiEcosystem::Rust => "rust",
                            PackageApiEcosystem::Go => "go",
                            PackageApiEcosystem::Java => "java",
                            PackageApiEcosystem::Csharp => "csharp",
                            PackageApiEcosystem::Native => "native",
                        },
                        surface.value.package.name
                    ));
                }
            }
            let referenced: BTreeSet<String> = self
                .consumer
                .references
                .iter()
                .map(|reference| {
                    format!(
                        "{}:{}",
                        match reference.value.target.package.ecosystem {
                            PackageApiEcosystem::Python => "python",
                            PackageApiEcosystem::Javascript => "javascript",
                            PackageApiEcosystem::Rust => "rust",
                            PackageApiEcosystem::Go => "go",
                            PackageApiEcosystem::Java => "java",
                            PackageApiEcosystem::Csharp => "csharp",
                            PackageApiEcosystem::Native => "native",
                        },
                        reference.value.target.package.name
                    )
                })
                .collect();
            for coord in provider_coords {
                if !referenced.contains(&coord) {
                    limitations.push(
                        CompatibilityLimitation::coded(
                            CompatibilityLimitationCode::ConsumerMissingImporterRefs,
                        )
                        .with_detail(format!("consumer_missing_importer_refs:{coord}")),
                    );
                }
            }
        }
        limitations.push(CompatibilityLimitation::coded(
            CompatibilityLimitationCode::BoundaryEqualityPolicyOnly,
        ));
        limitations.sort();
        limitations.dedup();
        providers.sort_by(|a, b| a.publication.cmp(&b.publication));
        Ok(CompatibilityResult {
            schema_version: PACKAGE_COMPATIBILITY_SCHEMA.into(),
            verdict: aggregate(&verdicts),
            consumer: self.consumer.receipt.clone(),
            providers,
            boundary_kind: request.boundary_kind,
            boundaries,
            sites,
            limitations,
        })
    }
}

fn aggregate(verdicts: &[Verdict]) -> Verdict {
    if verdicts.contains(&Verdict::BrokenAt) {
        Verdict::BrokenAt
    } else if verdicts.is_empty() || verdicts.contains(&Verdict::IncompleteBind) {
        Verdict::IncompleteBind
    } else if verdicts.contains(&Verdict::Unknown) {
        Verdict::Unknown
    } else if verdicts.contains(&Verdict::Bound) {
        Verdict::Bound
    } else {
        Verdict::Fits
    }
}

fn mapped_root_missing(
    limitations: &[CompatibilityLimitation],
    package: &PackageApiPackage,
) -> bool {
    limitations.iter().any(|gap| {
        gap.code == CompatibilityLimitationCode::MappedPackageRootMissing
            && gap.detail.as_deref() == Some(package.name.as_str())
    })
}

fn export_at<'a>(
    surface: &'a Surface,
    scheme: &str,
    spelling: &str,
) -> Option<(usize, &'a PackageApiExport)> {
    surface
        .export_index
        .get(&(scheme.to_string(), spelling.to_string()))
        .and_then(|&index| {
            surface
                .value
                .exports
                .get(index)
                .map(|export| (index, export))
        })
}

/// Seeking site: locate a unique provider export without requiring an assessed contract.
fn locate_unassessed_export(
    site: &mut CompatibilitySite,
    provider: &PackagePublication,
    surface: &Surface,
) {
    site.provider_publication = Some(provider.id().into());
    site.evidence.push(EvidenceRef {
        artifact_id: surface.artifact_id.clone(),
        provider_pointer: surface.pointer.clone(),
        record_id: None,
    });
    if let Some((index, export)) = export_at(
        surface,
        &site.target.symbol.scheme,
        &site.target.symbol.spelling,
    ) {
        site.evidence.push(EvidenceRef {
            artifact_id: surface.artifact_id.clone(),
            provider_pointer: format!("{}/exports/{index}", surface.pointer),
            record_id: None,
        });
        site.observed_boundary = Some(export.boundary_digest.clone());
        site.verdict = Verdict::Bound;
        site.reason = CompatibilityReason::ProviderExportLocatedContractUnassessed;
        return;
    }
    // Seeking sites never invent break from absence: that still needs an assessed
    // expected boundary. Incomplete menus stay explicit; complete menus stay seeking.
    if surface.value.completeness != ScopeCompleteness::CompleteRelativeToUniverse
        || !surface.complete_scope
    {
        site.reason = CompatibilityReason::ProviderExportScopeIncomplete;
        return;
    }
    site.reason = CompatibilityReason::ConsumerBoundaryUnassessed;
}

fn compare_site(site: &mut CompatibilitySite, provider: &PackagePublication, surface: &Surface) {
    site.provider_publication = Some(provider.id().into());
    site.evidence.push(EvidenceRef {
        artifact_id: surface.artifact_id.clone(),
        provider_pointer: surface.pointer.clone(),
        record_id: None,
    });
    if !surface.complete_scope {
        site.reason = CompatibilityReason::ProviderExportScopeIncomplete;
        return;
    }
    if site.boundary_policy == "unassessed" || site.expected_boundary.is_none() {
        site.reason = CompatibilityReason::ConsumerBoundaryUnassessed;
        return;
    }
    let Some((index, export)) = export_at(
        surface,
        &site.target.symbol.scheme,
        &site.target.symbol.spelling,
    ) else {
        if !surface.value.export_universes.iter().any(|universe| {
            universe.symbol_scheme == site.target.symbol.scheme
                && universe.boundary_policy == site.boundary_policy
        }) {
            site.verdict = Verdict::Unknown;
            site.reason = CompatibilityReason::ExportUniverseUnassessed;
            return;
        }
        // Enumeration completeness is needed for absence, not for a positive
        // declaration whose exact policy-qualified boundary is already present.
        if surface.value.completeness != ScopeCompleteness::CompleteRelativeToUniverse {
            site.reason = CompatibilityReason::ProviderExportScopeIncomplete;
            return;
        }
        site.verdict = Verdict::BrokenAt;
        site.reason = CompatibilityReason::ExportAbsentInCompleteSurface;
        return;
    };
    site.evidence.push(EvidenceRef {
        artifact_id: surface.artifact_id.clone(),
        provider_pointer: format!("{}/exports/{index}", surface.pointer),
        record_id: None,
    });
    site.observed_boundary = Some(export.boundary_digest.clone());
    if export.boundary_policy != site.boundary_policy {
        site.verdict = Verdict::Unknown;
        site.reason = CompatibilityReason::BoundaryPoliciesDiffer;
    } else if let Some(expected) = &site.expected_boundary {
        if expected == &export.boundary_digest {
            site.verdict = Verdict::Fits;
            site.reason = CompatibilityReason::ObservedBoundaryEqualBodyNotAssessed;
        } else {
            site.verdict = Verdict::BrokenAt;
            site.reason = CompatibilityReason::ObservedBoundaryChanged;
        }
    } else {
        site.reason = CompatibilityReason::ConsumerExpectedBoundaryMissing;
    }
}

pub use entmoot_evidence::package_inventory::validate_package_inventory;
