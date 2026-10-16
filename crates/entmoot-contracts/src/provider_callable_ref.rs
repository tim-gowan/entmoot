//! Structured provider callable reference (`logic.provider_callable_ref.v1`).
//!
//! Package identity uses Package URL (via the `packageurl` crate). In-package
//! callable ancestry uses a package-relative path. Native toolchain spelling stays
//! attributable and is never Entmoot LogicId. Repository file paths belong on
//! occurrences (`RepoPath` / inventory), not in this FQN.

use std::fmt::Write as _;
use std::str::FromStr;

use packageurl::PackageUrl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_CALLABLE_REF_SCHEMA: &str = "logic.provider_callable_ref.v1";

/// Attributable provider spelling for display or lookup.
///
/// Not Entmoot identity. Use conventional scheme strings such as
/// `python.qualified_name`, `jvm.method_descriptor`, `dotnet.metadata_name`,
/// `rust.def_path`, `cxx.usr`, or `{language}.compiler.symbol`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderSymbolSpelling {
    pub scheme: String,
    pub spelling: String,
}

const COMPACT_PREFIX: &str = "sym:1";

/// Schemes that must not appear as `native.scheme` (provider spelling ≠ LogicId).
const FORBIDDEN_NATIVE_SCHEMES: &[&str] = &[
    "logic_id",
    "logic_address",
    "address_id",
    "exact_body",
    "structural_hash",
    "resolved_hash",
    "body_hash",
];

/// How the package relates to the analyzed application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPackageRole {
    Stdlib,
    Module,
    Application,
    Unknown,
}

/// Coarse declaration kind for a provider callable reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallableKind {
    Callable,
    Type,
    Value,
    Module,
    Intrinsic,
    Unknown,
}

/// Callable path segment kind inside a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPathKind {
    Namespace,
    Type,
    Term,
    Method,
    Parameter,
    TypeParameter,
    Macro,
    Meta,
}

/// One ordered ancestry segment toward a callable (never a repository file path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderPathSegment {
    pub kind: ProviderPathKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disambiguator: Option<String>,
}

/// Package / distribution layer for a provider callable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderPackageRef {
    pub role: ProviderPackageRole,
    /// Canonical `pkg:…` string when known; validated via `packageurl`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecosystem: Option<String>,
}

/// Portable, attributable fully-qualified callable reference (not LogicId).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderCallableRef {
    pub schema_version: String,
    pub kind: ProviderCallableKind,
    pub package: ProviderPackageRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<ProviderPathSegment>,
    /// Human/debug rendering; not a stitch key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Lossless toolchain dialect.
    pub native: ProviderSymbolSpelling,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProviderCallableRefError {
    #[error("unsupported provider_callable_ref schema_version: {0}")]
    UnsupportedVersion(String),
    #[error("empty field: {0}")]
    EmptyField(&'static str),
    #[error("native.scheme must not be an Entmoot identity scheme: {0}")]
    ForbiddenNativeScheme(String),
    #[error("invalid package.purl: {0}")]
    InvalidPurl(String),
    #[error("compact encode/decode error: {0}")]
    Compact(String),
    #[error("purl build error: {0}")]
    PurlBuild(String),
}

impl ProviderCallableRef {
    pub fn validate(&self) -> Result<(), ProviderCallableRefError> {
        if self.schema_version != PROVIDER_CALLABLE_REF_SCHEMA {
            return Err(ProviderCallableRefError::UnsupportedVersion(
                self.schema_version.clone(),
            ));
        }
        if self.native.scheme.trim().is_empty() {
            return Err(ProviderCallableRefError::EmptyField("native.scheme"));
        }
        if self.native.spelling.trim().is_empty() {
            return Err(ProviderCallableRefError::EmptyField("native.spelling"));
        }
        if FORBIDDEN_NATIVE_SCHEMES.contains(&self.native.scheme.as_str()) {
            return Err(ProviderCallableRefError::ForbiddenNativeScheme(
                self.native.scheme.clone(),
            ));
        }
        if let Some(purl) = &self.package.purl {
            let parsed = PackageUrl::from_str(purl)
                .map_err(|error| ProviderCallableRefError::InvalidPurl(error.to_string()))?;
            let canonical = parsed.to_string();
            if canonical != *purl {
                return Err(ProviderCallableRefError::InvalidPurl(format!(
                    "non-canonical purl; expected {canonical}"
                )));
            }
        }
        if self
            .package
            .ecosystem
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ProviderCallableRefError::EmptyField("package.ecosystem"));
        }
        for segment in &self.path {
            if segment.name.trim().is_empty() {
                return Err(ProviderCallableRefError::EmptyField("path.name"));
            }
            if segment
                .disambiguator
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(ProviderCallableRefError::EmptyField("path.disambiguator"));
            }
        }
        Ok(())
    }
}

/// Build a canonical golang module Package URL via `packageurl`.
///
/// The pinned PURL normalizer lowercases namespaces. When that loses the native
/// module spelling, the Entmoot `go_module_path` qualifier preserves it. This is
/// an Entmoot coordinate policy, not an upstream PURL qualifier. Package API
/// decoding validates the qualifier against the base coordinate.
pub fn purl_golang_module(
    module_path: &str,
    version: Option<&str>,
) -> Result<String, ProviderCallableRefError> {
    let trimmed = module_path.trim();
    if trimmed.is_empty() {
        return Err(ProviderCallableRefError::EmptyField("golang.module_path"));
    }
    let (namespace, name) = split_go_module_path(trimmed);
    let mut purl = PackageUrl::new("golang", name)
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    if let Some(namespace) = namespace {
        purl.with_namespace(namespace)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    if purl.namespace() != namespace {
        purl.add_qualifier("go_module_path", trimmed)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    if let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) {
        purl.with_version(version)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    Ok(purl.to_string())
}

/// Build a canonical golang stdlib Package URL via `packageurl`.
pub fn purl_golang_stdlib(go_version: Option<&str>) -> Result<String, ProviderCallableRefError> {
    let mut purl = PackageUrl::new("golang", "stdlib")
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    if let Some(version) = go_version.map(str::trim).filter(|value| !value.is_empty()) {
        // Strip a leading "go" if present so `@1.22.0` stays conventional.
        let normalized = version.strip_prefix("go").unwrap_or(version);
        purl.with_version(normalized)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    Ok(purl.to_string())
}

/// Build a canonical npm Package URL via `packageurl` (scoped names supported).
pub fn purl_npm(name: &str, version: Option<&str>) -> Result<String, ProviderCallableRefError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProviderCallableRefError::EmptyField("npm.name"));
    }
    let (namespace, package_name) = split_npm_name(trimmed)?;
    let mut purl = PackageUrl::new("npm", package_name)
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    if let Some(namespace) = namespace {
        purl.with_namespace(namespace)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    if let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) {
        purl.with_version(version)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    Ok(purl.to_string())
}

/// Build a canonical PyPI Package URL via `packageurl`.
pub fn purl_pypi(name: &str, version: Option<&str>) -> Result<String, ProviderCallableRefError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProviderCallableRefError::EmptyField("pypi.name"));
    }
    let mut purl = PackageUrl::new("pypi", trimmed)
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    if let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) {
        purl.with_version(version)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    Ok(purl.to_string())
}

/// Build a canonical cargo Package URL via `packageurl`.
pub fn purl_cargo(name: &str, version: Option<&str>) -> Result<String, ProviderCallableRefError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProviderCallableRefError::EmptyField("cargo.name"));
    }
    let mut purl = PackageUrl::new("cargo", trimmed)
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    if let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) {
        purl.with_version(version)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    Ok(purl.to_string())
}

/// Build a canonical maven Package URL via `packageurl`.
pub fn purl_maven(
    group: &str,
    artifact: &str,
    version: Option<&str>,
) -> Result<String, ProviderCallableRefError> {
    let group = group.trim();
    let artifact = artifact.trim();
    if group.is_empty() {
        return Err(ProviderCallableRefError::EmptyField("maven.group"));
    }
    if artifact.is_empty() {
        return Err(ProviderCallableRefError::EmptyField("maven.artifact"));
    }
    let mut purl = PackageUrl::new("maven", artifact)
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    purl.with_namespace(group)
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    if let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) {
        purl.with_version(version)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    Ok(purl.to_string())
}

/// Build a canonical nuget Package URL via `packageurl`.
pub fn purl_nuget(name: &str, version: Option<&str>) -> Result<String, ProviderCallableRefError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProviderCallableRefError::EmptyField("nuget.name"));
    }
    let mut purl = PackageUrl::new("nuget", trimmed)
        .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    if let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) {
        purl.with_version(version)
            .map_err(|error| ProviderCallableRefError::PurlBuild(error.to_string()))?;
    }
    Ok(purl.to_string())
}

/// Encode a structured ref into a compact interchange string (derived; not SoR).
pub fn encode_provider_callable_ref(
    value: &ProviderCallableRef,
) -> Result<String, ProviderCallableRefError> {
    value.validate()?;
    let ecosystem = value
        .package
        .ecosystem
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("_");
    let purl = value
        .package
        .purl
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("_");
    let path = encode_path(&value.path)?;
    let mut out = String::new();
    write!(
        &mut out,
        "{COMPACT_PREFIX} {} {} {}",
        escape_field(ecosystem),
        escape_field(purl),
        path
    )
    .map_err(|error| ProviderCallableRefError::Compact(error.to_string()))?;
    Ok(out)
}

/// Decoded compact-form fields before callers fill kind/role/native.
pub type DecodedProviderCallableParts = (Option<String>, Option<String>, Vec<ProviderPathSegment>);

/// Decode a compact interchange string into a structured ref skeleton.
///
/// Restores `package.ecosystem`, `package.purl`, and `path`. Callers must fill
/// `kind`, `package.role`, `native`, and optional `display` from context.
pub fn decode_provider_callable_ref(
    compact: &str,
) -> Result<DecodedProviderCallableParts, ProviderCallableRefError> {
    let trimmed = compact.trim();
    let rest = trimmed
        .strip_prefix(COMPACT_PREFIX)
        .ok_or_else(|| ProviderCallableRefError::Compact("missing sym:1 prefix".into()))?
        .trim_start();
    let mut parts = split_fields(rest);
    if parts.len() != 3 {
        return Err(ProviderCallableRefError::Compact(format!(
            "expected 3 fields after prefix, got {}",
            parts.len()
        )));
    }
    let ecosystem = unescape_field(parts.remove(0))?;
    let purl = unescape_field(parts.remove(0))?;
    let path = decode_path(&parts.remove(0))?;
    let ecosystem = if ecosystem == "_" {
        None
    } else {
        Some(ecosystem)
    };
    let purl = if purl == "_" { None } else { Some(purl) };
    if let Some(purl_value) = &purl {
        let _ = PackageUrl::from_str(purl_value)
            .map_err(|error| ProviderCallableRefError::InvalidPurl(error.to_string()))?;
    }
    Ok((ecosystem, purl, path))
}

fn split_go_module_path(module_path: &str) -> (Option<&str>, &str) {
    match module_path.rsplit_once('/') {
        Some((namespace, name)) if !namespace.is_empty() && !name.is_empty() => {
            (Some(namespace), name)
        }
        _ => (None, module_path),
    }
}

fn split_npm_name(name: &str) -> Result<(Option<String>, String), ProviderCallableRefError> {
    if let Some(rest) = name.strip_prefix('@') {
        let (scope, package) = rest.split_once('/').ok_or_else(|| {
            ProviderCallableRefError::PurlBuild("scoped npm package requires @scope/name".into())
        })?;
        if scope.is_empty() || package.is_empty() {
            return Err(ProviderCallableRefError::PurlBuild(
                "scoped npm package requires @scope/name".into(),
            ));
        }
        Ok((Some(format!("@{scope}")), package.to_owned()))
    } else {
        Ok((None, name.to_owned()))
    }
}

fn encode_path(path: &[ProviderPathSegment]) -> Result<String, ProviderCallableRefError> {
    let mut out = String::new();
    for segment in path {
        let name = escape_segment(&segment.name);
        let disambiguator = segment
            .disambiguator
            .as_deref()
            .map(escape_segment)
            .unwrap_or_default();
        match segment.kind {
            ProviderPathKind::Namespace => {
                write!(&mut out, "{name}/")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
            ProviderPathKind::Type => {
                write!(&mut out, "{name}#")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
            ProviderPathKind::Term => {
                write!(&mut out, "{name}.")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
            ProviderPathKind::Method => {
                write!(&mut out, "{name}({disambiguator}).")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
            ProviderPathKind::Parameter => {
                write!(&mut out, "({name})")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
            ProviderPathKind::TypeParameter => {
                write!(&mut out, "[{name}]")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
            ProviderPathKind::Macro => {
                write!(&mut out, "{name}!")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
            ProviderPathKind::Meta => {
                write!(&mut out, "`{name}`")
                    .map_err(|e| ProviderCallableRefError::Compact(e.to_string()))?;
            }
        }
    }
    Ok(out)
}

fn decode_path(raw: &str) -> Result<Vec<ProviderPathSegment>, ProviderCallableRefError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut path = Vec::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            let end = find_closing(&chars, i, '(', ')')?;
            let name = unescape_segment(&chars[i + 1..end].iter().collect::<String>())?;
            path.push(ProviderPathSegment {
                kind: ProviderPathKind::Parameter,
                name,
                disambiguator: None,
            });
            i = end + 1;
            continue;
        }
        if chars[i] == '[' {
            let end = find_closing(&chars, i, '[', ']')?;
            let name = unescape_segment(&chars[i + 1..end].iter().collect::<String>())?;
            path.push(ProviderPathSegment {
                kind: ProviderPathKind::TypeParameter,
                name,
                disambiguator: None,
            });
            i = end + 1;
            continue;
        }
        if chars[i] == '`' {
            let end = chars[i + 1..]
                .iter()
                .position(|c| *c == '`')
                .map(|offset| i + 1 + offset)
                .ok_or_else(|| ProviderCallableRefError::Compact("unclosed meta".into()))?;
            let name = unescape_segment(&chars[i + 1..end].iter().collect::<String>())?;
            path.push(ProviderPathSegment {
                kind: ProviderPathKind::Meta,
                name,
                disambiguator: None,
            });
            i = end + 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !matches!(chars[i], '/' | '#' | '.' | '!' | '(' | '[' | '`') {
            i += 1;
        }
        if i >= chars.len() {
            return Err(ProviderCallableRefError::Compact(
                "path segment missing kind terminator".into(),
            ));
        }
        let name_raw: String = chars[start..i].iter().collect();
        let terminator = chars[i];
        i += 1;
        match terminator {
            '/' => path.push(ProviderPathSegment {
                kind: ProviderPathKind::Namespace,
                name: unescape_segment(&name_raw)?,
                disambiguator: None,
            }),
            '#' => path.push(ProviderPathSegment {
                kind: ProviderPathKind::Type,
                name: unescape_segment(&name_raw)?,
                disambiguator: None,
            }),
            '!' => path.push(ProviderPathSegment {
                kind: ProviderPathKind::Macro,
                name: unescape_segment(&name_raw)?,
                disambiguator: None,
            }),
            '.' => path.push(ProviderPathSegment {
                kind: ProviderPathKind::Term,
                name: unescape_segment(&name_raw)?,
                disambiguator: None,
            }),
            '(' => {
                let end = find_closing(&chars, i - 1, '(', ')')?;
                let disambiguator_raw: String = chars[i..end].iter().collect();
                if end + 1 >= chars.len() || chars[end + 1] != '.' {
                    return Err(ProviderCallableRefError::Compact(
                        "method segment must end with ).".into(),
                    ));
                }
                let disambiguator = if disambiguator_raw.is_empty() {
                    None
                } else {
                    Some(unescape_segment(&disambiguator_raw)?)
                };
                path.push(ProviderPathSegment {
                    kind: ProviderPathKind::Method,
                    name: unescape_segment(&name_raw)?,
                    disambiguator,
                });
                i = end + 2;
            }
            _ => {
                return Err(ProviderCallableRefError::Compact(format!(
                    "unexpected path terminator {terminator}"
                )));
            }
        }
    }
    Ok(path)
}

fn find_closing(
    chars: &[char],
    open_at: usize,
    open: char,
    close: char,
) -> Result<usize, ProviderCallableRefError> {
    let mut depth = 0;
    for (idx, ch) in chars.iter().enumerate().skip(open_at) {
        if *ch == open {
            depth += 1;
        } else if *ch == close {
            depth -= 1;
            if depth == 0 {
                return Ok(idx);
            }
        }
    }
    Err(ProviderCallableRefError::Compact(format!(
        "unclosed {open}{close}"
    )))
}

fn escape_field(value: &str) -> String {
    value.replace('%', "%25").replace(' ', "%20")
}

fn unescape_field(value: String) -> Result<String, ProviderCallableRefError> {
    unescape_percent(&value)
}

fn escape_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '%' => out.push_str("%25"),
            ' ' => out.push_str("%20"),
            '/' => out.push_str("%2F"),
            '#' => out.push_str("%23"),
            '.' => out.push_str("%2E"),
            '!' => out.push_str("%21"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '[' => out.push_str("%5B"),
            ']' => out.push_str("%5D"),
            '`' => out.push_str("%60"),
            other => out.push(other),
        }
    }
    out
}

fn unescape_segment(value: &str) -> Result<String, ProviderCallableRefError> {
    unescape_percent(value)
}

fn unescape_percent(value: &str) -> Result<String, ProviderCallableRefError> {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(ProviderCallableRefError::Compact(
                    "truncated percent escape".into(),
                ));
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .map_err(|error| ProviderCallableRefError::Compact(error.to_string()))?;
            let decoded = u8::from_str_radix(hex, 16)
                .map_err(|error| ProviderCallableRefError::Compact(error.to_string()))?;
            out.push(char::from(decoded));
            i += 3;
        } else {
            out.push(char::from(bytes[i]));
            i += 1;
        }
    }
    Ok(out)
}

fn split_fields(raw: &str) -> Vec<String> {
    raw.splitn(3, ' ').map(str::to_owned).collect()
}

/// Derive a compact display string from path segments (not a stitch key).
pub fn display_from_path(path: &[ProviderPathSegment]) -> String {
    let mut parts = Vec::new();
    for segment in path {
        match segment.kind {
            ProviderPathKind::Namespace | ProviderPathKind::Type | ProviderPathKind::Term => {
                parts.push(segment.name.clone());
            }
            ProviderPathKind::Method => {
                if let Some(disambiguator) = &segment.disambiguator {
                    parts.push(format!("{}({})", segment.name, disambiguator));
                } else {
                    parts.push(segment.name.clone());
                }
            }
            ProviderPathKind::Parameter
            | ProviderPathKind::TypeParameter
            | ProviderPathKind::Macro
            | ProviderPathKind::Meta => parts.push(segment.name.clone()),
        }
    }
    parts.join(".")
}

/// Split supported Go function/method spellings into package, receiver and leaf.
/// Generic or unfamiliar spellings remain only in the lossless native field.
pub fn path_from_go_qualified_name(symbol: &str) -> Vec<ProviderPathSegment> {
    let trimmed = symbol.trim();
    if trimmed.is_empty() || trimmed.contains(['[', ']']) {
        return Vec::new();
    }
    let Some((owner, name)) = trimmed.rsplit_once('.') else {
        return Vec::new();
    };
    let identifier = |value: &str| {
        let mut chars = value.chars();
        chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
            && chars.all(|c| c == '_' || c.is_alphanumeric())
    };
    if !identifier(name) {
        return Vec::new();
    }
    let (package, receiver) =
        if let Some(receiver) = owner.strip_prefix('(').and_then(|v| v.strip_suffix(')')) {
            let receiver = receiver.strip_prefix('*').unwrap_or(receiver);
            let Some((package, ty)) = receiver.rsplit_once('.') else {
                return Vec::new();
            };
            if !identifier(ty) {
                return Vec::new();
            }
            (package, Some(ty))
        } else {
            (owner, None)
        };
    if package.is_empty() || package.contains(['(', ')', '*', ' ', '\t', '\n']) {
        return Vec::new();
    }
    // An import path is one namespace, not a repository path or dotted ancestry.
    let mut path = vec![ProviderPathSegment {
        kind: ProviderPathKind::Namespace,
        name: package.to_owned(),
        disambiguator: None,
    }];
    if let Some(receiver) = receiver {
        path.push(ProviderPathSegment {
            kind: ProviderPathKind::Type,
            name: receiver.to_owned(),
            disambiguator: None,
        });
    }
    path.push(ProviderPathSegment {
        kind: ProviderPathKind::Method,
        name: name.to_owned(),
        disambiguator: None,
    });
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_receivers_keep_type_and_import_path_distinct() {
        for symbol in [
            "(net/http.Handler).ServeHTTP",
            "(*net/http.Handler).ServeHTTP",
        ] {
            let path = path_from_go_qualified_name(symbol);
            assert_eq!(
                path.iter()
                    .map(|p| (p.kind, p.name.as_str()))
                    .collect::<Vec<_>>(),
                vec![
                    (ProviderPathKind::Namespace, "net/http"),
                    (ProviderPathKind::Type, "Handler"),
                    (ProviderPathKind::Method, "ServeHTTP"),
                ]
            );
        }
        let path = path_from_go_qualified_name("example.com/v2/pkg.Function");
        assert_eq!(path[0].name, "example.com/v2/pkg");
        assert!(path_from_go_qualified_name("(*example.com/pkg.Set[other.Type]).Add").is_empty());
        assert!(path_from_go_qualified_name("(unqualified).Method").is_empty());
    }

    fn native(scheme: &str, spelling: &str) -> ProviderSymbolSpelling {
        ProviderSymbolSpelling {
            scheme: scheme.into(),
            spelling: spelling.into(),
        }
    }

    #[test]
    fn golang_and_npm_purls_use_packageurl_canonical_form() {
        assert_eq!(
            purl_golang_module("github.com/gorilla/mux", Some("v1.8.0")).unwrap(),
            "pkg:golang/github.com/gorilla/mux@v1.8.0"
        );
        assert_eq!(
            purl_golang_stdlib(Some("go1.22.0")).unwrap(),
            "pkg:golang/stdlib@1.22.0"
        );
        assert_eq!(
            purl_npm("@angular/animation", Some("12.3.1")).unwrap(),
            "pkg:npm/%40angular/animation@12.3.1"
        );
        assert_eq!(purl_npm("lodash", None).unwrap(), "pkg:npm/lodash");
    }

    #[test]
    fn encode_decode_round_trips_go_fmt_errorf() {
        let path = path_from_go_qualified_name("fmt.Errorf");
        let value = ProviderCallableRef {
            schema_version: PROVIDER_CALLABLE_REF_SCHEMA.into(),
            kind: ProviderCallableKind::Callable,
            package: ProviderPackageRef {
                role: ProviderPackageRole::Stdlib,
                purl: Some(purl_golang_stdlib(Some("1.22.0")).unwrap()),
                ecosystem: Some("go".into()),
            },
            path: path.clone(),
            display: Some(display_from_path(&path)),
            native: native("go.compiler.symbol", "fmt.Errorf"),
        };
        value.validate().unwrap();
        let compact = encode_provider_callable_ref(&value).unwrap();
        let (ecosystem, purl, decoded_path) = decode_provider_callable_ref(&compact).unwrap();
        assert_eq!(ecosystem.as_deref(), Some("go"));
        assert_eq!(purl.as_deref(), Some("pkg:golang/stdlib@1.22.0"));
        assert_eq!(decoded_path, path);
    }

    #[test]
    fn encode_decode_round_trips_scoped_npm_export() {
        let path = vec![
            ProviderPathSegment {
                kind: ProviderPathKind::Namespace,
                name: "@docsearch/js".into(),
                disambiguator: None,
            },
            ProviderPathSegment {
                kind: ProviderPathKind::Method,
                name: "docsearch".into(),
                disambiguator: None,
            },
        ];
        let value = ProviderCallableRef {
            schema_version: PROVIDER_CALLABLE_REF_SCHEMA.into(),
            kind: ProviderCallableKind::Callable,
            package: ProviderPackageRef {
                role: ProviderPackageRole::Module,
                purl: Some(purl_npm("@docsearch/js", Some("4.5.3")).unwrap()),
                ecosystem: Some("javascript".into()),
            },
            path: path.clone(),
            display: Some(display_from_path(&path)),
            native: native("javascript.compiler.symbol", "@docsearch/js#docsearch"),
        };
        let compact = encode_provider_callable_ref(&value).unwrap();
        let (_, purl, decoded_path) = decode_provider_callable_ref(&compact).unwrap();
        assert_eq!(purl.as_deref(), Some("pkg:npm/%40docsearch/js@4.5.3"));
        assert_eq!(decoded_path, path);
    }

    #[test]
    fn rejects_entmoot_identity_native_scheme() {
        let value = ProviderCallableRef {
            schema_version: PROVIDER_CALLABLE_REF_SCHEMA.into(),
            kind: ProviderCallableKind::Callable,
            package: ProviderPackageRef {
                role: ProviderPackageRole::Unknown,
                purl: None,
                ecosystem: None,
            },
            path: vec![],
            display: None,
            native: native("logic_address", "x"),
        };
        assert!(matches!(
            value.validate(),
            Err(ProviderCallableRefError::ForbiddenNativeScheme(_))
        ));
    }
}
