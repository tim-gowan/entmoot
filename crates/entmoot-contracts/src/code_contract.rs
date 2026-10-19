//! Source-declared objects at relationship boundaries, without type resolution.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{SiteSpan, SourceOccurrenceRef};

/// Durable source-declared boundary. The envelope pins the exact publication,
/// source bytes and occurrence independently of disposable topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeContractRecord {
    pub record_id: String,
    pub snapshot: crate::SourceSnapshot,
    pub path: String,
    pub content_digest: crate::ContentDigest,
    pub contract: CodeContract,
}

impl CodeContractRecord {
    pub fn derive_id(&self) -> String {
        crate::hash_parts(&[
            "entmoot.code_contract_record.v1",
            &format!("{:?}", self.snapshot),
            &self.path,
            self.content_digest.as_str(),
            self.contract.site.occurrence.address_id.as_str(),
            &self.contract.site.span.start_byte.to_string(),
            &self.contract.site.span.end_byte.to_string(),
            // Full attributed observation, including optional presence, variants,
            // method and every source extent. boundary_key intentionally omits
            // relocation detail and is unsuitable for durable record identity.
            &format!("{:?}", self.contract),
        ])
    }

    pub fn validate(&self) -> Result<(), String> {
        crate::paths::validate_source_observation(
            &self.snapshot,
            &self.path,
            &self.content_digest,
        )?;
        self.contract.validate()?;
        if self.record_id != self.derive_id() {
            return Err("code contract record identity mismatch".into());
        }
        Ok(())
    }
}

/// A written type expression in its language's syntax. No nominal equivalence,
/// inferred type, alias resolution or runtime value is implied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeclaredType {
    Explicit {
        syntax: String,
        span: SiteSpan,
    },
    /// No type was written at this position; this does not mean void or any.
    Unspecified,
    /// Syntax exists but the extractor cannot isolate a type safely.
    Unsupported {
        syntax: String,
        span: SiteSpan,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    Ordinary,
    PositionalOnly,
    KeywordOnly,
    Variadic,
    KeywordVariadic,
    Receiver,
    Unsupported,
}

/// An unevaluated expression with its exact UTF-8 source extent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredExpression {
    pub syntax: String,
    pub span: SiteSpan,
}

/// One declaration parameter, not an argument-to-parameter binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredParameter {
    /// Written declaration qualifiers (including optional `?` syntax), without
    /// interpretation across languages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: ParameterKind,
    pub span: SiteSpan,
    pub declared_type: DeclaredType,
    /// Written default expression, not an evaluated default value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<DeclaredExpression>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    Field,
    Property,
    Method,
    Embedded,
    Unsupported,
}

/// Directly declared member only; no inheritance or runtime shape closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredMember {
    /// Written member qualifiers such as visibility, static or optional syntax.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: MemberKind,
    pub span: SiteSpan,
    pub declared_type: DeclaredType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeclaredContract {
    Callable {
        parameters: Vec<DeclaredParameter>,
        result: DeclaredType,
    },
    Object {
        members: Vec<DeclaredMember>,
    },
    Unsupported,
}

/// Parser observation attached to an actual production occurrence. Snapshot,
/// path and source digest are supplied by its inventory publication envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeContract {
    pub method: String,
    pub language: String,
    pub name: String,
    pub site: SourceOccurrenceRef,
    pub shape: DeclaredContract,
    /// Bounded extraction gaps, never evidence of member/type absence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}

impl CodeContract {
    pub const METHOD: &'static str = "entmoot.syntax.code_contract.v1";

    pub fn validate(&self) -> Result<(), String> {
        self.site.validate()?;
        if self.method != Self::METHOD || self.name.is_empty() || self.language.is_empty() {
            return Err("invalid code contract producer context".into());
        }
        let span = |s: SiteSpan| {
            if s.start_byte < self.site.span.start_byte
                || s.end_byte > self.site.span.end_byte
                || s.start_byte >= s.end_byte
            {
                Err("code contract span outside its occurrence".to_owned())
            } else {
                Ok(())
            }
        };
        let ty = |t: &DeclaredType| -> Result<(), String> {
            match t {
                DeclaredType::Unspecified => Ok(()),
                DeclaredType::Explicit { syntax, span: s }
                | DeclaredType::Unsupported { syntax, span: s } => {
                    span(*s)?;
                    if syntax.is_empty() || syntax.len() != s.end_byte - s.start_byte {
                        return Err("type syntax does not match its byte extent".into());
                    }
                    Ok(())
                }
            }
        };
        match &self.shape {
            DeclaredContract::Callable { parameters, result } => {
                for p in parameters {
                    span(p.span)?;
                    ty(&p.declared_type)?;
                    if let Some(value) = &p.default {
                        span(value.span)?;
                        if value.syntax.is_empty()
                            || value.syntax.len() != value.span.end_byte - value.span.start_byte
                        {
                            return Err("default syntax does not match its byte extent".into());
                        }
                    }
                }
                ty(result)?;
            }
            DeclaredContract::Object { members } => {
                for m in members {
                    span(m.span)?;
                    ty(&m.declared_type)?;
                }
            }
            DeclaredContract::Unsupported if self.limitations.is_empty() => {
                return Err("unsupported contract requires a limitation".into())
            }
            DeclaredContract::Unsupported => {}
        }
        Ok(())
    }

    /// Compare declared shape while ignoring relocation coordinates. Retains
    /// exact type spelling; this is not semantic type equivalence.
    pub fn boundary_key(&self) -> String {
        fn ty(t: &DeclaredType) -> (&str, &str) {
            match t {
                DeclaredType::Explicit { syntax, .. } => ("explicit", syntax),
                DeclaredType::Unsupported { syntax, .. } => ("unsupported", syntax),
                DeclaredType::Unspecified => ("unspecified", ""),
            }
        }
        match &self.shape {
            DeclaredContract::Callable { parameters, result } => format!(
                "callable:{:?}:{:?}:{:?}:{:?}",
                self.language,
                parameters
                    .iter()
                    .map(|p| (
                        &p.name,
                        p.kind,
                        &p.modifiers,
                        ty(&p.declared_type),
                        p.default.as_ref().map(|d| &d.syntax)
                    ))
                    .collect::<Vec<_>>(),
                ty(result),
                self.limitations
            ),
            DeclaredContract::Object { members } => format!(
                "object:{:?}:{:?}:{:?}",
                self.language,
                members
                    .iter()
                    .map(|m| (&m.name, m.kind, &m.modifiers, ty(&m.declared_type)))
                    .collect::<Vec<_>>(),
                self.limitations
            ),
            DeclaredContract::Unsupported => {
                format!("unsupported:{:?}:{:?}", self.language, self.limitations)
            }
        }
    }
}
