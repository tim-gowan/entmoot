//! Attributed package selections -> validated identity/digests -> exact origin join.
//! Lock entry meaning belongs to the emitting producer, never to this adapter.
use super::pipeline::NativeProducerError;
use entmoot_contracts::{
    package_api::{LockedPackageInstance, PackageApiPackage},
    ContentDigest,
};
use entmoot_contracts::{CapturedOutput, NativeRawObservation};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

/// Local producer envelope, accepted either from .entmoot/lock_bindings.json
/// (operator/CI provider) or native build_context.lock_bindings. Physical roots
/// stay in retained native evidence; only LockedPackageInstance is portable.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LockAssertions {
    /// Attribution, not authentication or authority granted by the engine.
    producer: String,
    /// Versioned interpretation policy owned by this producer.
    policy: String,
    packages: Vec<PackageSelection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageSelection {
    /// Physical native source directory or compiler-selected archive input.
    root: PathBuf,
    /// Already selected by the producer; the engine never derives coordinates.
    instance: LockedPackageInstance,
}

struct FrozenInput {
    bytes: Vec<u8>,
    digest: ContentDigest,
}

pub(super) struct LockInputs {
    root: PathBuf,
    config_absent: bool,
    inputs: BTreeMap<PathBuf, FrozenInput>,
    paths: BTreeMap<PathBuf, (PathBuf, bool)>,
    selected: BTreeMap<PathBuf, Vec<(bool, LockedPackageInstance)>>,
    placements: BTreeSet<String>,
    retained_bytes: u64,
}

fn invalid(detail: impl std::fmt::Display) -> NativeProducerError {
    NativeProducerError::Artifact(format!("invalid lock selection evidence: {detail}"))
}

/// Minimal `uv.lock` [[package]] name/version extractor (no TOML dependency).
fn parse_uv_lock_packages(bytes: &[u8]) -> Result<Vec<(String, String)>, NativeProducerError> {
    let text = std::str::from_utf8(bytes).map_err(invalid)?;
    let mut packages = Vec::new();
    let mut in_package = false;
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if let (Some(n), Some(v)) = (name.take(), version.take()) {
                packages.push((n, v));
            }
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            if let (Some(n), Some(v)) = (name.take(), version.take()) {
                packages.push((n, v));
            }
            in_package = false;
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name = ") {
            name = Some(rest.trim().trim_matches('"').to_owned());
        } else if let Some(rest) = trimmed.strip_prefix("version = ") {
            version = Some(rest.trim().trim_matches('"').to_owned());
        }
    }
    if let (Some(n), Some(v)) = (name, version) {
        packages.push((n, v));
    }
    Ok(packages)
}

fn read_uv_lock_assertions(root: &Path) -> Result<Option<LockAssertions>, NativeProducerError> {
    let lock = root.join("uv.lock");
    match std::fs::symlink_metadata(&lock) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(meta) if !meta.is_file() => return Ok(None),
        Ok(_) => {}
    }
    let bytes = std::fs::read(&lock)?;
    let digest = ContentDigest::from_trusted(entmoot_contracts::blake3_hex(&bytes));
    let packages = parse_uv_lock_packages(&bytes)?;
    if packages.is_empty() {
        return Ok(None);
    }
    let mut rows = Vec::new();
    for (name, version) in packages {
        // Placement is unique per locked name@version (uv locks may list the same
        // name under multiple resolution markers).
        let placement = format!(".entmoot/uv-lock/{name}@{version}");
        let purl = format!("pkg:pypi/{name}@{version}");
        let mut instance = LockedPackageInstance {
            instance_id: String::new(),
            purl,
            lock_path: "uv.lock".into(),
            lock_digest: digest.clone(),
            entry: name.clone(),
            placement,
        };
        instance.instance_id = instance.derive_id();
        if instance.validate().is_err() {
            continue;
        }
        rows.push(PackageSelection {
            root: PathBuf::from("."),
            instance,
        });
    }
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(LockAssertions {
        producer: "entmoot.uv_lock".into(),
        policy: "uv.lock.package.v1".into(),
        packages: rows,
    }))
}

impl LockInputs {
    pub(super) fn prepare(root: &Path) -> Result<Self, NativeProducerError> {
        let mut result = Self {
            root: root.to_owned(),
            config_absent: false,
            inputs: BTreeMap::new(),
            paths: BTreeMap::new(),
            selected: BTreeMap::new(),
            placements: BTreeSet::new(),
            retained_bytes: 0,
        };
        let file = root.join(".entmoot/lock_bindings.json");
        match std::fs::symlink_metadata(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                result.config_absent = true;
                // Optional uv.lock assertion hook: strengthen Fits when package_api
                // references match locked PyPI coordinates. Never invent install roots.
                if let Some(assertions) = read_uv_lock_assertions(root)? {
                    result.accept(assertions)?;
                }
                return Ok(result);
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let path = result.track_path(&file)?;
        if !path.starts_with(root) {
            return Err(invalid("operator input escapes consumer"));
        }
        let config: LockAssertions = serde_json::from_slice(&result.read(&path)?.bytes)?;
        result.accept(config)?;
        Ok(result)
    }

    fn track_path(&mut self, path: &Path) -> Result<PathBuf, NativeProducerError> {
        let physical = path.canonicalize()?;
        let observed = (physical.clone(), physical.is_dir());
        if self.paths.get(path).is_some_and(|prior| *prior != observed) {
            return Err(NativeProducerError::Drift(format!(
                "selection path changed: {}",
                path.display()
            )));
        }
        self.paths.insert(path.to_owned(), observed);
        Ok(physical)
    }

    /// The only selection admission path, shared by the operator and native emitters.
    fn accept(&mut self, assertions: LockAssertions) -> Result<(), NativeProducerError> {
        if assertions.producer.trim().is_empty()
            || assertions.policy.trim().is_empty()
            || assertions.producer.len() > 4096
            || assertions.policy.len() > 4096
        {
            return Err(invalid("producer and versioned policy are required"));
        }
        if self
            .placements
            .len()
            .saturating_add(assertions.packages.len())
            > 4096
        {
            return Err(invalid("package budget"));
        }
        for row in assertions.packages {
            let instance = row.instance;
            instance.validate().map_err(invalid)?;
            if !self.placements.insert(instance.placement.clone()) {
                return Err(invalid("duplicate installation placement"));
            }
            let path = self.root.join(&instance.lock_path);
            let lock_path = self.track_path(&path)?;
            if !lock_path.starts_with(&self.root) {
                return Err(invalid("lock input escapes consumer"));
            }
            if self.read(&lock_path)?.digest != instance.lock_digest {
                return Err(invalid("cited lock digest does not match retained bytes"));
            }
            let physical = self.track_path(&self.root.join(row.root))?;
            let directory = physical.is_dir();
            self.selected
                .entry(physical)
                .or_default()
                .push((directory, instance));
        }
        Ok(())
    }

    fn read(&mut self, path: &Path) -> Result<&FrozenInput, NativeProducerError> {
        if !self.inputs.contains_key(path) {
            let length = std::fs::metadata(path)?.len();
            if length > 32 * 1024 * 1024
                || self.retained_bytes.saturating_add(length) > 64 * 1024 * 1024
            {
                return Err(invalid("input byte budget"));
            }
            let bytes = read_bounded(path, length + 1)?;
            if bytes.len() as u64 != length {
                return Err(NativeProducerError::Drift(
                    "lock input length changed during read".into(),
                ));
            }
            self.retained_bytes += length;
            let digest = ContentDigest::from_trusted(entmoot_contracts::blake3_hex(&bytes));
            self.inputs
                .insert(path.to_owned(), FrozenInput { bytes, digest });
        }
        self.inputs
            .get(path)
            .ok_or_else(|| invalid("input missing"))
    }

    pub(super) fn annotate(
        mut self,
        raw: &mut NativeRawObservation,
    ) -> Result<(), NativeProducerError> {
        // A naked reference field lacks the physical placement evidence required
        // by this collector. Normalized Resolver artifacts retain their own port.
        if raw
            .build_context
            .get("package_api")
            .and_then(|v| v.get("references"))
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.get("lock_instance").is_some_and(|v| !v.is_null()))
            })
        {
            return Err(invalid(
                "emit selections through build_context.lock_bindings",
            ));
        }
        if let Some(bundle) = raw.build_context.get("lock_bindings") {
            self.accept(serde_json::from_value(bundle.clone())?)?;
        }
        if self.config_absent {
            match std::fs::symlink_metadata(self.root.join(".entmoot/lock_bindings.json")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
                Ok(_) => {
                    return Err(NativeProducerError::Drift(
                        "operator selection input appeared during collection".into(),
                    ))
                }
            }
        }
        for (path, (physical, directory)) in &self.paths {
            if path.canonicalize()? != *physical || physical.is_dir() != *directory {
                return Err(NativeProducerError::Drift(format!(
                    "selection path changed: {}",
                    path.display()
                )));
            }
        }
        // Operator inputs were frozen before collection. Producer-emitted inputs
        // are checked against the producer's digest after collection, then rechecked.
        // Neither fence claims a transactional filesystem or package authenticity.
        for (path, input) in &self.inputs {
            if read_bounded(path, input.bytes.len() as u64 + 1)? != input.bytes {
                return Err(NativeProducerError::Drift(format!(
                    "lock input changed: {}",
                    path.display()
                )));
            }
            raw.inputs.push(path.to_string_lossy().into_owned());
        }
        // Absent selection evidence must leave the producer context unchanged.
        if !self.inputs.is_empty() {
            raw.build_context.insert(
                "lock_binding_inputs".into(),
                json!(self
                    .inputs
                    .into_iter()
                    .map(|(path, input)| (
                        path.to_string_lossy().into_owned(),
                        json!({"digest":input.digest, "content":CapturedOutput::from(input.bytes)})
                    ))
                    .collect::<BTreeMap<_, _>>()),
            );
        }
        if self.selected.is_empty() || raw.invalid_environment || raw.budget_exceeded {
            return Ok(());
        }
        let mut calls = BTreeMap::<_, Vec<_>>::new();
        for call in &raw.calls {
            calls
                .entry((
                    call.site.path.as_str(),
                    call.site.span.start_byte,
                    call.site.span.end_byte,
                ))
                .or_default()
                .push(call);
        }
        let Some(api) = raw.build_context.get_mut("package_api") else {
            return Ok(());
        };
        let Some(references) = api.get_mut("references").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        let mut origins = BTreeMap::<String, Option<PathBuf>>::new();
        for reference in references.iter_mut() {
            if reference.get("lock_instance").is_some_and(|v| !v.is_null()) {
                continue;
            }
            let Some(key) = reference["path"]
                .as_str()
                .zip(reference["start_byte"].as_u64())
                .zip(reference["end_byte"].as_u64())
                .and_then(|((p, s), e)| {
                    Some((p, usize::try_from(s).ok()?, usize::try_from(e).ok()?))
                })
            else {
                continue;
            };
            let Some(candidates) = calls.get(&key) else {
                continue;
            };
            let [call] = candidates.as_slice() else {
                continue;
            };
            if call.gap.is_some() {
                continue;
            }
            let [target] = call.targets.as_slice() else {
                continue;
            };
            let Some(file) = target.location.origin_file() else {
                continue;
            };
            let Some(origin) = origins
                .entry(file.to_owned())
                .or_insert_with(|| Path::new(file).canonicalize().ok())
                .as_ref()
            else {
                continue;
            };
            let candidates: Vec<_> = origin
                .ancestors()
                .filter_map(|root| self.selected.get(root).map(|rows| (root, rows)))
                .flat_map(|(root, rows)| {
                    rows.iter().filter_map(move |(directory, instance)| {
                        (*directory || root == origin).then_some(instance)
                    })
                })
                .take(2)
                .collect();
            let [instance] = candidates.as_slice() else {
                continue;
            };
            let package: PackageApiPackage =
                serde_json::from_value(reference["target"]["package"].clone())?;
            if PackageApiPackage::from_purl(&instance.purl).map_err(invalid)? != Some(package) {
                continue;
            }
            if let Some(version) = &target.package_version {
                // Package URL parsing below compares exact native version bytes.
                use std::str::FromStr;
                let purl = packageurl::PackageUrl::from_str(&instance.purl).map_err(invalid)?;
                if purl.version() != Some(version.as_str()) {
                    continue;
                }
            }
            reference["lock_instance"] = serde_json::to_value(instance)?;
        }
        // Seeking / external refs lack origin_file. Attach when exactly one
        // asserted selection projects to the reference package coordinates.
        // Do not fall back when an origin_file was present but failed to join
        // (wrong version, outside root, …)—that would erase those fences.
        let mut by_package: BTreeMap<PackageApiPackage, Vec<&LockedPackageInstance>> =
            BTreeMap::new();
        for rows in self.selected.values() {
            for (_, instance) in rows {
                let Some(package) =
                    PackageApiPackage::from_purl(&instance.purl).map_err(invalid)?
                else {
                    continue;
                };
                by_package.entry(package).or_default().push(instance);
            }
        }
        for reference in references {
            if reference.get("lock_instance").is_some_and(|v| !v.is_null()) {
                continue;
            }
            let Some(key) = reference["path"]
                .as_str()
                .zip(reference["start_byte"].as_u64())
                .zip(reference["end_byte"].as_u64())
                .and_then(|((p, s), e)| {
                    Some((p, usize::try_from(s).ok()?, usize::try_from(e).ok()?))
                })
            else {
                continue;
            };
            let Some(candidates) = calls.get(&key) else {
                continue;
            };
            let [call] = candidates.as_slice() else {
                continue;
            };
            if call.gap.is_some() {
                continue;
            }
            let [target] = call.targets.as_slice() else {
                continue;
            };
            if target.location.origin_file().is_some() {
                continue;
            }
            let Ok(package) =
                serde_json::from_value::<PackageApiPackage>(reference["target"]["package"].clone())
            else {
                continue;
            };
            let Some(candidates) = by_package.get(&package) else {
                continue;
            };
            let [instance] = candidates.as_slice() else {
                continue;
            };
            if let Some(version) = &target.package_version {
                use std::str::FromStr;
                let purl = packageurl::PackageUrl::from_str(&instance.purl).map_err(invalid)?;
                if purl.version() != Some(version.as_str()) {
                    continue;
                }
            }
            reference["lock_instance"] = serde_json::to_value(instance)?;
        }
        Ok(())
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, NativeProducerError> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_freshness_and_cross_provider_placements_cannot_be_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join(".entmoot")).unwrap();
        std::fs::write(root.join("selection.lock"), b"opaque").unwrap();
        std::fs::write(root.join("binary"), b"native archive").unwrap();
        let mut instance = LockedPackageInstance {
            instance_id: String::new(),
            purl: "pkg:nuget/Api@1.0.0".into(),
            lock_path: "selection.lock".into(),
            lock_digest: ContentDigest::from_trusted(entmoot_contracts::blake3_hex(b"opaque")),
            entry: "a".into(),
            placement: "libs/api".into(),
        };
        instance.instance_id = instance.derive_id();
        let config = json!({"producer":"operator","policy":"selection.v1",
            "packages":[{"root":"binary","instance":instance}]});
        let path = root.join(".entmoot/lock_bindings.json");
        let absent = LockInputs::prepare(&root).unwrap();
        std::fs::write(&path, config.to_string()).unwrap();
        assert!(absent
            .annotate(&mut NativeRawObservation::default())
            .is_err());
        let frozen = LockInputs::prepare(&root).unwrap();
        std::fs::write(&path, format!("{config}\n")).unwrap();
        assert!(frozen
            .annotate(&mut NativeRawObservation::default())
            .is_err());
        let mut raw = NativeRawObservation::default();
        raw.build_context.insert("lock_bindings".into(), config);
        assert!(LockInputs::prepare(&root)
            .unwrap()
            .annotate(&mut raw)
            .is_err());

        // An archive root qualifies only that exact native origin.
        let package = PackageApiPackage::from_purl(&instance.purl)
            .unwrap()
            .unwrap();
        let mut raw: NativeRawObservation = serde_json::from_value(json!({"calls":[{
            "site":{"path":"caller","span":{"start_byte":1,"end_byte":10}},
            "targets":[{"location":{"kind":"unmapped","origin_file":root.join("binary"),"span":{"start_byte":0,"end_byte":5}},"symbol":"Api.Leaf"}]
        }]})).unwrap();
        raw.build_context.insert(
            "package_api".into(),
            json!({"references":[{
                "path":"caller","start_byte":1,"end_byte":10,"target":{"package":package}
            }]}),
        );
        LockInputs::prepare(&root)
            .unwrap()
            .annotate(&mut raw)
            .unwrap();
        assert_eq!(
            raw.build_context["package_api"]["references"][0]["lock_instance"],
            json!(instance)
        );
    }

    #[test]
    fn every_claimed_family_uses_opaque_attributed_selections_and_exact_origins() {
        // Shared port controls, NOT ecosystem lock/parser qualification.
        for purl in [
            "pkg:cargo/api@1.0.0",
            "pkg:pypi/api@1.0.0",
            "pkg:npm/api@1.0.0",
            "pkg:golang/example.org/Api@v1.0.0",
            "pkg:maven/org.example/api@1.0.0",
            "pkg:nuget/Api@1.0.0",
            "pkg:conan/api@1.0.0",
        ] {
            for provider in ["operator", "native"] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().canonicalize().unwrap();
                std::fs::create_dir(root.join(".entmoot")).unwrap();
                std::fs::create_dir(root.join("dependency")).unwrap();
                std::fs::write(root.join("dependency/api"), "native declaration").unwrap();
                let lock = b"\xffselected bytes\x00";
                std::fs::write(root.join("selected.lock"), lock).unwrap();
                let mut instance = LockedPackageInstance {
                    instance_id: String::new(),
                    purl: purl.into(),
                    lock_path: "selected.lock".into(),
                    lock_digest: ContentDigest::from_trusted(entmoot_contracts::blake3_hex(lock)),
                    entry: "opaque producer locator".into(),
                    placement: "deps/api".into(),
                };
                instance.instance_id = instance.derive_id();
                let row = json!({"root":"dependency", "instance":instance});
                let config_path = root.join(".entmoot/lock_bindings.json");
                let package = PackageApiPackage::from_purl(purl).unwrap().unwrap();
                let source = || {
                    let mut raw: NativeRawObservation = serde_json::from_value(json!({"calls":[{
                        "site":{"path":"caller", "span":{"start_byte":1,"end_byte":10}},"targets":[{
                            "location":{"kind":"unmapped","origin_file":root.join("dependency/api"),"span":{"start_byte":0,"end_byte":5}},"symbol":"api.leaf"
                        }]
                    }]})).unwrap();
                    raw.build_context.insert("package_api".into(), json!({"references":[{
                        "path":"caller","start_byte":1,"end_byte":10,"target":{"package":package}
                    }]}));
                    raw
                };
                let configure = |rows: Vec<Value>, raw: &mut NativeRawObservation| {
                    let value = json!({"producer":"fixture.operator", "policy":"fixture.selected.v1", "packages":rows});
                    if provider == "operator" {
                        std::fs::write(&config_path, value.to_string()).unwrap();
                    } else {
                        raw.build_context.insert("lock_bindings".into(), value);
                    }
                };
                let selected = |raw: &NativeRawObservation| {
                    raw.build_context["package_api"]["references"][0]
                        .get("lock_instance")
                        .cloned()
                };
                let mut raw = source();
                configure(vec![row.clone()], &mut raw);
                LockInputs::prepare(&root)
                    .unwrap()
                    .annotate(&mut raw)
                    .unwrap();
                assert_eq!(
                    selected(&raw).unwrap(),
                    json!(instance),
                    "{purl} {provider}"
                );
                assert!(raw.build_context["lock_binding_inputs"]
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|v| v["content"]["encoding"] == "bytes"
                        && v["content"]["value"] == json!(lock)));
                for mode in [
                    "absent_origin",
                    "outside_origin",
                    "invalid_environment",
                    "dynamic",
                    "budget",
                    "duplicate_call",
                    "multiple_targets",
                    "wrong_version",
                    "wrong_package",
                ] {
                    let mut raw = source();
                    match mode {
                        "absent_origin" => {
                            raw.calls[0].targets[0].location =
                                entmoot_contracts::NativeTargetLocation::Unmapped {
                                    span: *raw.calls[0].targets[0].location.span(),
                                    origin_file: None,
                                }
                        }
                        "outside_origin" => {
                            raw.calls[0].targets[0].location =
                                entmoot_contracts::NativeTargetLocation::Unmapped {
                                    span: *raw.calls[0].targets[0].location.span(),
                                    origin_file: Some(
                                        root.join("selected.lock").to_string_lossy().into(),
                                    ),
                                }
                        }
                        "invalid_environment" => raw.invalid_environment = true,
                        "budget" => raw.budget_exceeded = true,
                        "duplicate_call" => raw.calls.push(raw.calls[0].clone()),
                        "multiple_targets" => {
                            let target = raw.calls[0].targets[0].clone();
                            raw.calls[0].targets.push(target);
                        }
                        "wrong_version" => {
                            raw.calls[0].targets[0].package_version = Some("99".into())
                        }
                        "wrong_package" => {
                            raw.build_context.get_mut("package_api").unwrap()["references"][0]
                                ["target"]["package"]["name"] = "other".into()
                        }
                        _ => raw.calls[0].gap = Some("dynamic_dispatch".into()),
                    }
                    configure(vec![row.clone()], &mut raw);
                    LockInputs::prepare(&root)
                        .unwrap()
                        .annotate(&mut raw)
                        .unwrap();
                    if mode == "absent_origin" {
                        // External/seeking calls join by unique package coordinates.
                        assert_eq!(
                            selected(&raw).unwrap(),
                            json!(instance),
                            "{purl} {provider}: {mode}"
                        );
                    } else {
                        assert!(selected(&raw).is_none(), "{purl} {provider}: {mode}");
                    }
                }
                for root_value in ["dependency", "."] {
                    let mut alias = instance.clone();
                    alias.placement = "deps/alias".into();
                    alias.instance_id = alias.derive_id();
                    let mut raw = source();
                    configure(
                        vec![row.clone(), json!({"root":root_value,"instance":alias})],
                        &mut raw,
                    );
                    LockInputs::prepare(&root)
                        .unwrap()
                        .annotate(&mut raw)
                        .unwrap();
                    assert!(
                        selected(&raw).is_none(),
                        "{purl} {provider}: overlapping roots"
                    );
                }
                for mode in [
                    "wrong_id",
                    "wrong_digest",
                    "noncanonical_path",
                    "duplicate",
                    "missing_producer",
                    "missing_bytes",
                    "budget",
                ] {
                    let mut changed = instance.clone();
                    match mode {
                        "wrong_id" => changed.instance_id = "invalid".into(),
                        "wrong_digest" => {
                            changed.lock_digest = ContentDigest::from_trusted(
                                entmoot_contracts::blake3_hex(b"other"),
                            );
                            changed.instance_id = changed.derive_id();
                        }
                        "noncanonical_path" => {
                            changed.lock_path = "../outside.lock".into();
                            changed.instance_id = changed.derive_id();
                        }
                        _ => {}
                    }
                    let mut raw = source();
                    let changed_row = json!({"root":"dependency","instance":changed});
                    configure(
                        if mode == "duplicate" {
                            vec![changed_row.clone(), changed_row]
                        } else {
                            vec![changed_row]
                        },
                        &mut raw,
                    );
                    if mode == "missing_producer" {
                        if provider == "operator" {
                            std::fs::write(
                                &config_path,
                                json!({"packages":[row.clone()]}).to_string(),
                            )
                            .unwrap();
                        } else {
                            raw.build_context
                                .insert("lock_bindings".into(), json!({"packages":[row.clone()]}));
                        }
                    }
                    if mode == "missing_bytes" {
                        std::fs::remove_file(root.join("selected.lock")).unwrap();
                    }
                    if mode == "budget" {
                        std::fs::File::create(root.join("selected.lock"))
                            .unwrap()
                            .set_len(32 * 1024 * 1024 + 1)
                            .unwrap();
                    }
                    assert!(
                        LockInputs::prepare(&root)
                            .and_then(|inputs| inputs.annotate(&mut raw))
                            .is_err(),
                        "{purl} {provider}: {mode}"
                    );
                    std::fs::write(root.join("selected.lock"), lock).unwrap();
                }
                let mut raw = source();
                configure(vec![row.clone()], &mut raw);
                let frozen = LockInputs::prepare(&root).unwrap();
                std::fs::write(root.join("selected.lock"), b"changed").unwrap();
                assert!(
                    frozen.annotate(&mut raw).is_err(),
                    "{purl} {provider}: drift"
                );
                std::fs::write(root.join("selected.lock"), lock).unwrap();
                if config_path.exists() {
                    std::fs::remove_file(&config_path).unwrap();
                }
                let mut raw = source();
                raw.build_context.get_mut("package_api").unwrap()["references"][0]
                    ["lock_instance"] = json!(instance);
                assert!(
                    LockInputs::prepare(&root)
                        .unwrap()
                        .annotate(&mut raw)
                        .is_err(),
                    "reference bypass"
                );
                let mut raw = source();
                let original = raw.clone();
                LockInputs::prepare(&root)
                    .unwrap()
                    .annotate(&mut raw)
                    .unwrap();
                assert!(selected(&raw).is_none(), "{purl}: no assertion");
                assert_eq!(
                    raw, original,
                    "{purl}: absent selection must preserve native output"
                );
            }
        }
    }

    #[test]
    fn unique_package_match_attaches_without_origin_file_and_ambiguous_stays_open() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join(".entmoot")).unwrap();
        std::fs::create_dir(root.join("dependency")).unwrap();
        std::fs::write(root.join("dependency/api"), "native declaration").unwrap();
        let lock = b"selected";
        std::fs::write(root.join("selected.lock"), lock).unwrap();
        let mut instance = LockedPackageInstance {
            instance_id: String::new(),
            purl: "pkg:npm/accepts@2.0.0".into(),
            lock_path: "selected.lock".into(),
            lock_digest: ContentDigest::from_trusted(entmoot_contracts::blake3_hex(lock)),
            entry: "node_modules/accepts".into(),
            placement: "node_modules/accepts".into(),
        };
        instance.instance_id = instance.derive_id();
        let package = PackageApiPackage::from_purl(&instance.purl)
            .unwrap()
            .unwrap();
        let config = json!({"producer":"fixture.operator","policy":"fixture.selected.v1",
            "packages":[{"root":"dependency","instance":instance}]});
        std::fs::write(root.join(".entmoot/lock_bindings.json"), config.to_string()).unwrap();
        let mut raw: NativeRawObservation = serde_json::from_value(json!({"calls":[{
            "site":{"path":"app.js","span":{"start_byte":1,"end_byte":20}},
            "targets":[{"location":{"kind":"unmapped","span":{"start_byte":0,"end_byte":0}},"symbol":"accepts#default",
                "module_path":"accepts","ecosystem":"javascript","kind":"callable"}]
        }]}))
        .unwrap();
        raw.build_context.insert(
            "package_api".into(),
            json!({"references":[{"path":"app.js","start_byte":1,"end_byte":20,
                "target":{"package":package},"boundary_policy":"unassessed"}]}),
        );
        LockInputs::prepare(&root)
            .unwrap()
            .annotate(&mut raw)
            .unwrap();
        assert_eq!(
            raw.build_context["package_api"]["references"][0]["lock_instance"],
            json!(instance)
        );

        let mut alias = instance.clone();
        alias.placement = "node_modules/accepts-alias".into();
        alias.instance_id = alias.derive_id();
        let config = json!({"producer":"fixture.operator","policy":"fixture.selected.v1",
        "packages":[
            {"root":"dependency","instance":instance},
            {"root":"dependency","instance":alias}
        ]});
        std::fs::write(root.join(".entmoot/lock_bindings.json"), config.to_string()).unwrap();
        let mut raw: NativeRawObservation = serde_json::from_value(json!({"calls":[{
            "site":{"path":"app.js","span":{"start_byte":1,"end_byte":20}},
            "targets":[{"location":{"kind":"unmapped","span":{"start_byte":0,"end_byte":0}},"symbol":"accepts#default",
                "module_path":"accepts","ecosystem":"javascript","kind":"callable"}]
        }]}))
        .unwrap();
        raw.build_context.insert(
            "package_api".into(),
            json!({"references":[{"path":"app.js","start_byte":1,"end_byte":20,
                "target":{"package":package},"boundary_policy":"unassessed"}]}),
        );
        LockInputs::prepare(&root)
            .unwrap()
            .annotate(&mut raw)
            .unwrap();
        assert!(raw.build_context["package_api"]["references"][0]
            .get("lock_instance")
            .is_none());
    }
}
