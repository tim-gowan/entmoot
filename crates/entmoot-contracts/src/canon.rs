//! Bounded canonical forms are model-relative evidence, never proof certificates.
use crate::{ContentDigest, SiteSpan};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CANON_SCHEMA: &str = "entmoot.canon.v1";
pub const CANON_NUDGE: &str = "Clone analysis requires a compatible canon graph. Run `entmoot bind --experimental canon` for this snapshot, then retry.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonSort {
    Bool,
    Integer { bits: Option<u16>, signed: bool },
    Unknown,
}

impl CanonSort {
    pub fn validate(&self) -> Result<(), CanonError> {
        if matches!(
            self,
            Self::Integer {
                bits: Some(0 | 129..),
                ..
            } | Self::Integer {
                bits: None,
                signed: false
            }
        ) {
            return Err(CanonError::Invalid("integer domain"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonOverflow {
    Wrap,
    Checked,
    Undefined,
    Unbounded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonOp {
    Input {
        slot: u32,
    },
    Bool {
        value: bool,
    },
    Integer {
        value: String,
    },
    Add {
        overflow: CanonOverflow,
    },
    Subtract {
        overflow: CanonOverflow,
    },
    Multiply {
        overflow: CanonOverflow,
    },
    Equal,
    Less,
    Not,
    And,
    Or,
    Ite,
    /// Opaque source commitment; never eligible for a complete canonical match.
    Opaque {
        source: ContentDigest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonTerm {
    pub digest: ContentDigest,
    pub op: CanonOp,
    pub sort: CanonSort,
    pub children: Vec<ContentDigest>,
}

fn digest(parts: &[&str]) -> ContentDigest {
    ContentDigest::from_trusted(format!("blake3:{}", crate::hash_parts(parts)))
}

impl CanonTerm {
    pub fn new(op: CanonOp, sort: CanonSort, children: Vec<ContentDigest>) -> Self {
        let mut term = Self {
            digest: digest(&[]),
            op,
            sort,
            children,
        };
        term.digest = term.content_digest();
        term
    }
    fn content_digest(&self) -> ContentDigest {
        // Explicit field framing; no source coordinates or process-local IDs.
        let sort = match &self.sort {
            CanonSort::Bool => "bool".into(),
            CanonSort::Unknown => "unknown".into(),
            CanonSort::Integer { bits, signed } => format!(
                "integer:{}:{signed}",
                bits.map_or("unbounded".into(), |n| n.to_string())
            ),
        };
        let overflow = |mode: &CanonOverflow| match mode {
            CanonOverflow::Wrap => "wrap",
            CanonOverflow::Checked => "checked",
            CanonOverflow::Undefined => "undefined",
            CanonOverflow::Unbounded => "unbounded",
        };
        let op = match &self.op {
            CanonOp::Input { slot } => format!("input:{slot}"),
            CanonOp::Bool { value } => format!("bool:{value}"),
            CanonOp::Integer { value } => format!("integer:{value}"),
            CanonOp::Add { overflow: m } => format!("add:{}", overflow(m)),
            CanonOp::Subtract { overflow: m } => format!("subtract:{}", overflow(m)),
            CanonOp::Multiply { overflow: m } => format!("multiply:{}", overflow(m)),
            CanonOp::Equal => "equal".into(),
            CanonOp::Less => "less".into(),
            CanonOp::Not => "not".into(),
            CanonOp::And => "and".into(),
            CanonOp::Or => "or".into(),
            CanonOp::Ite => "ite".into(),
            CanonOp::Opaque { source } => format!("opaque:{source}"),
        };
        let mut parts = vec![CANON_SCHEMA, "term", &sort, &op];
        parts.extend(self.children.iter().map(ContentDigest::as_str));
        digest(&parts)
    }
    pub fn validate(&self) -> Result<(), CanonError> {
        self.sort.validate()?;
        if self.digest != self.content_digest() {
            return Err(CanonError::Invalid("term digest"));
        }
        let arity = match self.op {
            CanonOp::Input { .. }
            | CanonOp::Bool { .. }
            | CanonOp::Integer { .. }
            | CanonOp::Opaque { .. } => 0,
            CanonOp::Not => 1,
            CanonOp::Ite => 3,
            _ => 2,
        };
        if self.children.len() != arity {
            return Err(CanonError::Invalid("term arity"));
        }
        if let CanonSort::Integer {
            bits: Some(bits), ..
        } = self.sort
        {
            if bits == 0 || bits > 128 {
                return Err(CanonError::Invalid("integer width"));
            }
        }
        if let CanonOp::Integer { value } = &self.op {
            let digits = value.strip_prefix('-').unwrap_or(value);
            if digits.is_empty()
                || !digits.bytes().all(|b| b.is_ascii_digit())
                || digits.len() > 256
                || (digits.len() > 1 && digits.starts_with('0'))
                || value == "-0"
            {
                return Err(CanonError::Invalid("integer spelling"));
            }
            let CanonSort::Integer { bits, signed } = self.sort else {
                return Err(CanonError::Invalid("integer sort"));
            };
            if let Some(bits) = bits {
                let in_range = if signed {
                    value.parse::<i128>().is_ok_and(|n| {
                        bits == 128 || (n >= -(1i128 << (bits - 1)) && n < (1i128 << (bits - 1)))
                    })
                } else {
                    value
                        .parse::<u128>()
                        .is_ok_and(|n| bits == 128 || n < (1u128 << bits))
                };
                if !in_range {
                    return Err(CanonError::Invalid("integer range"));
                }
            }
        }
        if matches!(self.op, CanonOp::Bool { .. }) && self.sort != CanonSort::Bool {
            return Err(CanonError::Invalid("boolean sort"));
        }
        if let CanonOp::Opaque { source } = &self.op {
            ContentDigest::parse(source.as_str())
                .map_err(|_| CanonError::Invalid("opaque source"))?;
        }
        for child in &self.children {
            ContentDigest::parse(child.as_str())
                .map_err(|_| CanonError::Invalid("child digest"))?;
        }
        Ok(())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CanonLimit {
    IndirectCall,
    UnresolvedBinding,
    ConflictingEvidence,
    UnsupportedConstruct,
    UnsupportedSemantics,
    InvalidEnvironment,
    BudgetExceeded,
}

impl CanonLimit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IndirectCall => "indirect_call",
            Self::UnresolvedBinding => "unresolved_binding",
            Self::ConflictingEvidence => "conflicting_evidence",
            Self::UnsupportedConstruct => "unsupported_construct",
            Self::UnsupportedSemantics => "unsupported_semantics",
            Self::InvalidEnvironment => "invalid_environment",
            Self::BudgetExceeded => "budget_exceeded",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CanonAssumption {
    /// Only scalar return values and modeled definedness are observable.
    ScalarValueObservations,
    /// The selected compiler/typechecker model applies to the invocation.
    ValidNativeProgram,
    /// Callers supply exact primitive runtime types (no subclasses or wrappers),
    /// in positional slot order. Annotations alone do not guarantee this.
    InputsRespectNativeTypes,
}

/// Ingredients for a future obligation, never a solver result or proof certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonObligation {
    pub digest: ContentDigest,
    pub result: ContentDigest,
    pub assumptions: Vec<CanonAssumption>,
    pub conditions: Vec<ContentDigest>,
}

impl CanonObligation {
    pub fn new(
        result: ContentDigest,
        assumptions: Vec<CanonAssumption>,
        conditions: Vec<ContentDigest>,
    ) -> Self {
        let mut value = Self {
            digest: digest(&[]),
            result,
            assumptions,
            conditions,
        };
        value.digest = value.content_digest();
        value
    }
    fn content_digest(&self) -> ContentDigest {
        let mut parts = vec![
            CANON_SCHEMA,
            "obligation",
            self.result.as_str(),
            "assumptions",
        ];
        parts.extend(self.assumptions.iter().map(|a| match a {
            CanonAssumption::ScalarValueObservations => "scalar_value_observations",
            CanonAssumption::ValidNativeProgram => "valid_native_program",
            CanonAssumption::InputsRespectNativeTypes => "inputs_respect_native_types",
        }));
        parts.push("conditions");
        parts.extend(self.conditions.iter().map(ContentDigest::as_str));
        digest(&parts)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonExpressionFact {
    pub site: crate::NativeSpanRef,
    pub source_digest: ContentDigest,
    pub sort: CanonSort,
    /// Present only when the provider qualified primitive integer arithmetic.
    pub overflow: Option<CanonOverflow>,
    pub limitations: Vec<CanonLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonForm {
    pub owner: String,
    pub span: SiteSpan,
    pub language: String,
    /// Written result boundary is retained until native conversion lowering qualifies its erasure.
    pub result_boundary: String,
    pub root: ContentDigest,
    /// Child-before-parent DAG; each term appears once within this form.
    pub terms: Vec<CanonTerm>,
    pub limitations: Vec<CanonLimit>,
    pub obligation: CanonObligation,
    pub inputs: Option<crate::PropertyInputManifest>,
}

impl CanonForm {
    pub fn fingerprint(&self) -> ContentDigest {
        let mut parts = vec![
            CANON_SCHEMA,
            "form",
            self.language.as_str(),
            self.result_boundary.as_str(),
            self.root.as_str(),
            self.obligation.digest.as_str(),
            "limitations",
        ];
        parts.extend(self.limitations.iter().map(|v| v.as_str()));
        parts.push("inputs");
        // Unused parameters still belong to the callable's modeled input domain.
        parts.extend(
            self.terms
                .iter()
                .filter(|t| matches!(t.op, CanonOp::Input { .. }))
                .map(|t| t.digest.as_str()),
        );
        digest(&parts)
    }
    pub fn complete(&self) -> bool {
        self.limitations.is_empty()
    }
    pub fn validate(&self) -> Result<(), CanonError> {
        if self.owner.is_empty()
            || self.language.is_empty()
            || self.span.start_byte > self.span.end_byte
            || self.terms.len() > 4096
        {
            return Err(CanonError::Invalid("form bounds"));
        }
        let mut seen = BTreeMap::<&str, &CanonTerm>::new();
        let mut input_count = 0;
        for term in &self.terms {
            term.validate()?;
            if let CanonOp::Input { slot } = term.op {
                if slot != input_count {
                    return Err(CanonError::Invalid("input slots"));
                }
                input_count += 1;
            }
            if self.complete()
                && matches!(
                    term.op,
                    CanonOp::Add {
                        overflow: CanonOverflow::Checked | CanonOverflow::Undefined
                    } | CanonOp::Subtract {
                        overflow: CanonOverflow::Checked | CanonOverflow::Undefined
                    } | CanonOp::Multiply {
                        overflow: CanonOverflow::Checked | CanonOverflow::Undefined
                    }
                )
            {
                return Err(CanonError::Invalid("unmodeled overflow"));
            }
            if term
                .children
                .iter()
                .any(|child| !seen.contains_key(child.as_str()))
                || seen.contains_key(term.digest.as_str())
            {
                return Err(CanonError::Invalid("term DAG"));
            }
            if self.complete()
                && (matches!(term.op, CanonOp::Opaque { .. }) || term.sort == CanonSort::Unknown)
            {
                return Err(CanonError::Invalid("opaque complete form"));
            }
            let children: Vec<_> = term
                .children
                .iter()
                .map(|id| &seen[id.as_str()].sort)
                .collect();
            let valid = match &term.op {
                CanonOp::Input { .. }
                | CanonOp::Bool { .. }
                | CanonOp::Integer { .. }
                | CanonOp::Opaque { .. } => true,
                CanonOp::Add { overflow }
                | CanonOp::Subtract { overflow }
                | CanonOp::Multiply { overflow } => {
                    matches!(term.sort, CanonSort::Integer { .. })
                        && children.iter().all(|s| **s == term.sort)
                        && (matches!(term.sort, CanonSort::Integer { bits: None, .. })
                            == (*overflow == CanonOverflow::Unbounded))
                }
                CanonOp::Equal => {
                    term.sort == CanonSort::Bool
                        && children[0] == children[1]
                        && *children[0] != CanonSort::Unknown
                }
                CanonOp::Less => {
                    term.sort == CanonSort::Bool
                        && children[0] == children[1]
                        && matches!(children[0], CanonSort::Integer { .. })
                }
                CanonOp::Not | CanonOp::And | CanonOp::Or => {
                    term.sort == CanonSort::Bool && children.iter().all(|s| **s == CanonSort::Bool)
                }
                CanonOp::Ite => {
                    *children[0] == CanonSort::Bool
                        && children[1] == children[2]
                        && *children[1] == term.sort
                }
            };
            if !valid {
                return Err(CanonError::Invalid("term sorts"));
            }
            seen.insert(term.digest.as_str(), term);
        }
        if !seen.contains_key(self.root.as_str())
            || self.obligation.conditions.iter().any(|c| {
                seen.get(c.as_str())
                    .is_none_or(|t| t.sort != CanonSort::Bool)
            })
        {
            return Err(CanonError::Invalid("root or condition"));
        }
        if self.obligation.result != self.root
            || self.obligation.digest != self.obligation.content_digest()
            || self.obligation.assumptions.first()
                != Some(&CanonAssumption::ScalarValueObservations)
            || self.obligation.assumptions.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(CanonError::Invalid("obligation"));
        }
        if self.limitations.windows(2).any(|w| w[0] >= w[1]) {
            return Err(CanonError::Invalid("limitations order"));
        }
        if let Some(inputs) = &self.inputs {
            inputs
                .derive_key(&crate::PropertyDependencySpec {
                    property_schema_id: CANON_SCHEMA.into(),
                    requires_signature_hash: true,
                    additional_inputs: vec![],
                })
                .map_err(|_| CanonError::Invalid("property inputs"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonFile {
    pub path: String,
    pub language: String,
    pub source_digest: ContentDigest,
    /// Inputs of the pure file fold, excluding publication scope; native facts are fresh.
    pub input_digest: ContentDigest,
    pub patterns: Vec<crate::CloneOccurrence>,
    pub forms: Vec<CanonForm>,
    pub alpha_unavailable_callables: usize,
    pub limitations: Vec<CanonLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonFileRef {
    pub path: String,
    pub claim_id: crate::RelationClaimId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonNativeContext {
    pub producer: crate::NativeProducerContext,
    pub semantic_status: crate::NativeSemanticStatus,
    pub limitations: Vec<String>,
    pub compiler_version: Option<String>,
    pub environment_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonGraph {
    pub schema: String,
    pub implementation: crate::ImplementationFingerprint,
    pub snapshot: crate::SourceSnapshot,
    /// IDs of file claims; file shards use existing durable claim storage.
    pub files: Vec<CanonFileRef>,
    pub native_context: Vec<CanonNativeContext>,
    pub context: ContentDigest,
    pub limitations: Vec<CanonLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonObservation {
    Graph { graph: CanonGraph },
    File { file: CanonFile },
}

impl CanonObservation {
    pub fn digest(&self) -> Result<ContentDigest, serde_json::Error> {
        // The schema/derivation fingerprint fences changes to this typed encoding.
        Ok(digest(&[CANON_SCHEMA, &serde_json::to_string(self)?]))
    }
    pub fn validate(&self) -> Result<(), CanonError> {
        match self {
            Self::Graph { graph } => {
                if graph.schema != CANON_SCHEMA || graph.files.len() > 10_000 {
                    return Err(CanonError::Invalid("graph schema or bounds"));
                }
                graph
                    .snapshot
                    .validate()
                    .map_err(|_| CanonError::Invalid("snapshot"))?;
                if graph
                    .files
                    .iter()
                    .map(|file| file.path.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    != graph.files.len()
                {
                    return Err(CanonError::Invalid("duplicate file"));
                }
                for file in &graph.files {
                    if crate::normalize_repo_path(&file.path).as_deref() != Ok(file.path.as_str()) {
                        return Err(CanonError::Invalid("file reference path"));
                    }
                }
            }
            Self::File { file } => {
                if crate::normalize_repo_path(&file.path).as_deref() != Ok(file.path.as_str())
                    || file.patterns.len() > 100_000
                    || file.forms.len() > 100_000
                {
                    return Err(CanonError::Invalid("file path or bounds"));
                }
                ContentDigest::parse(file.source_digest.as_str())
                    .map_err(|_| CanonError::Invalid("source digest"))?;
                ContentDigest::parse(file.input_digest.as_str())
                    .map_err(|_| CanonError::Invalid("input digest"))?;
                if file
                    .patterns
                    .iter()
                    .any(|p| p.path != file.path || p.span.start_byte >= p.span.end_byte)
                {
                    return Err(CanonError::Invalid("pattern scope"));
                }
                for form in &file.forms {
                    form.validate()?;
                    if form.language != file.language {
                        return Err(CanonError::Invalid("form language"));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonAvailability {
    Available,
    Missing,
    Stale,
    Incompatible,
}

#[derive(Debug, thiserror::Error)]
pub enum CanonError {
    #[error("invalid canon {0}")]
    Invalid(&'static str),
}
