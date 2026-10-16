//! Per-file analysis markers emitted from the parsed syntax tree.
//!
//! Consumed by Phase 2 topology/indexer; optional on [`FileIndexRecord`](crate::FileIndexRecord).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Import statement classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ImportKind {
    Module,
    Named,
    Star,
    Static,
    Alias,
}

/// Binding established during alpha-rename or scope analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BindingKind {
    Parameter,
    Local,
    Global,
}

/// Kind of control-structure site (if/loop/match/return/…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ControlSiteKind {
    Conditional,
    Loop,
    Switch,
    Match,
    Return,
    Break,
    Continue,
    Throw,
}

/// File-level or module-level import site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImportMarker {
    /// Provider-independent semantic target named by the import.
    ///
    /// Module imports store the module path. Named/static imports store the
    /// complete target, including the imported member.
    pub path: String,
    /// Local source binding when it differs from the target's final segment,
    /// or when the import form (for example a JS default import) requires it.
    pub alias: Option<String>,
    pub kind: ImportKind,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Type, class, or callable definition site (logic kind as string: `function`, `method`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DefMarker {
    pub name: String,
    pub kind: String,
    pub scope: Vec<String>,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Variable or parameter binding discovered during alpha-renaming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BindingMarker {
    pub name: String,
    pub normalized: Option<String>,
    pub binding_kind: BindingKind,
    /// Declaration byte offset when the language visitor can provide it.
    /// Missing means unknown; zero is a valid source offset and is never a sentinel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<usize>,
}

/// Addressable declaration that is not itself an implementation occurrence, or
/// whose source binding is distinct from the callable implementation it names.
///
/// A class declaration has no `callable` target. An assigned callable (for
/// example `handler = lambda: ...`) points at the separately hashed callable
/// occurrence without conflating the binding span with the implementation span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclarationMarker {
    /// Optional bounded syntax observation; absence means not inventoried by this producer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax: Option<DeclarationSyntax>,
    pub name: String,
    /// Coarse syntax-owned kind such as `class`, `assigned_callable`, or `binding`.
    pub kind: String,
    pub scope: Vec<String>,
    pub site: crate::SourceOccurrenceRef,
    /// Complete implementation occurrence emitted by this file's structural visitor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callable: Option<crate::SourceOccurrenceRef>,
}

impl DeclarationMarker {
    pub fn validate(&self) -> Result<(), String> {
        self.site.validate()?;
        if self.name.is_empty() || self.kind.is_empty() {
            return Err("declaration requires a name and kind".into());
        }
        if (self.kind == "assigned_callable") != self.callable.is_some() {
            return Err(
                "assigned callable declarations require exactly one implementation reference"
                    .into(),
            );
        }
        if let Some(callable) = &self.callable {
            callable.validate()?;
            // An assigned declaration may span only its LHS identifier. The
            // implementation need not be contained by that declaration span.
            if callable.occurrence == self.site.occurrence {
                return Err(
                    "declaration callable must be a distinct implementation occurrence".into(),
                );
            }
        }
        if let Some(syntax) = &self.syntax {
            syntax.validate(self.site.span)?;
        }
        Ok(())
    }
}

/// A simple named binding and its explicitly written initializer. This is not
/// definite assignment, a read/write edge, a runtime default, or a property proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclarationSyntax {
    pub method: String,
    pub name_span: SiteSpan,
    /// None means no initializer syntax at this site, not an uninitialized value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initializer: Option<InitializerSyntax>,
}

/// Source-local initializer evidence. Expressions are never evaluated. Only
/// plain decimal integer syntax (at most 256 bytes, no suffix/radix/separators)
/// receives a canonical integer spelling; type/coercion and runtime value are unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitializerSyntax {
    pub span: SiteSpan,
    pub content_digest: crate::ContentDigest,
    pub value: InitializerValue,
}

/// Syntactic classification, never a runtime evaluation. The canonical decimal
/// spelling is derived from the committed text, not stored as a second value
/// which could disagree with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InitializerValue {
    Unevaluated {},
    DecimalInteger { text: String },
}

impl InitializerSyntax {
    /// Bounded mathematical spelling for exact syntax lookup; no type/coercion claim.
    pub fn integer(&self) -> Option<String> {
        match &self.value {
            InitializerValue::Unevaluated {} => None,
            InitializerValue::DecimalInteger { text } => decimal_integer(text),
        }
    }
}

impl DeclarationSyntax {
    pub const METHOD: &'static str = "entmoot.syntax.declaration_initializer.v1";

    pub fn validate(&self, span: SiteSpan) -> Result<(), String> {
        let contained = |s: SiteSpan| {
            span.start_byte <= s.start_byte
                && s.start_byte < s.end_byte
                && s.end_byte <= span.end_byte
        };
        if self.method != Self::METHOD || !contained(self.name_span) {
            return Err("invalid declaration syntax method or name span".into());
        }
        if let Some(init) = &self.initializer {
            if !contained(init.span) || init.span.start_byte < self.name_span.end_byte {
                return Err("initializer is outside declaration or precedes binding".into());
            }
            crate::ContentDigest::parse(init.content_digest.as_str()).map_err(|e| e.to_string())?;
            match &init.value {
                InitializerValue::Unevaluated {} => {}
                InitializerValue::DecimalInteger { text }
                    if decimal_integer(text).is_some()
                        && text.len() == init.span.end_byte - init.span.start_byte
                        && format!("blake3:{}", blake3::hash(text.as_bytes()).to_hex())
                            == init.content_digest.as_str() => {}
                _ => return Err("initializer literal disagrees with source commitment".into()),
            }
        }
        Ok(())
    }
}

/// Canonical mathematical spelling of bounded plain signed decimal syntax.
/// Leading zeroes, negative zero, suffixes and nondecimal spellings remain unknown.
pub fn decimal_integer(text: &str) -> Option<String> {
    if text.len() > 256 {
        return None;
    }
    let (negative, digits) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest.trim_start_matches([' ', '\t', '\r', '\n']))
    } else if let Some(rest) = text.strip_prefix('+') {
        (false, rest.trim_start_matches([' ', '\t', '\r', '\n']))
    } else {
        (false, text)
    };
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
        || (negative && digits == "0")
    {
        return None;
    }
    Some(format!("{}{digits}", if negative { "-" } else { "" }))
}

/// Durable declaration observation. Snapshot and full-file commitments scope
/// the evidence independently of disposable topology and callable identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclarationSiteRecord {
    pub site_id: String,
    pub snapshot: crate::SourceSnapshot,
    pub path: String,
    pub content_digest: crate::ContentDigest,
    pub marker: DeclarationMarker,
}

impl DeclarationSiteRecord {
    pub fn derive_id(&self) -> String {
        crate::hash_parts(&[
            DeclarationSyntax::METHOD,
            &self.snapshot.repository_id,
            &self.snapshot.revision,
            self.snapshot.snapshot_digest.as_deref().unwrap_or(""),
            self.snapshot.estate_id.as_deref().unwrap_or(""),
            &format!("{:?}", self.snapshot.source_state),
            &self.path,
            self.content_digest.as_str(),
            self.marker.site.occurrence.address_id.as_str(),
            &self.marker.site.span.start_byte.to_string(),
            &self.marker.site.span.end_byte.to_string(),
        ])
    }

    pub fn validate(&self) -> Result<(), String> {
        self.snapshot.validate().map_err(|e| e.to_string())?;
        if crate::normalize_repo_path(&self.path).map_err(|e| e.to_string())? != self.path {
            return Err("declaration path must be canonical".into());
        }
        crate::ContentDigest::parse(self.content_digest.as_str()).map_err(|e| e.to_string())?;
        self.marker.validate()?;
        if self.marker.name.is_empty()
            || self.marker.name.len() > 1024
            || self.site_id != self.derive_id()
            || self.marker.kind != "binding"
            || self.marker.callable.is_some()
        {
            return Err("invalid declaration site identity or shape".into());
        }
        self.marker
            .syntax
            .as_ref()
            .ok_or("missing declaration syntax")?
            .validate(self.marker.site.span)
    }
}

/// Control-structure site: AST span of a conditional/loop/switch/match/return/break/continue/throw.
/// Inventory annotation only — not a CFG edge (no successors, dominance, or SSA).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ControlSite {
    pub from_byte: usize,
    pub to_byte: usize,
    pub kind: ControlSiteKind,
}

/// Stable syntactic reference to the callable that owns a marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefinitionSiteRef {
    pub name: String,
    pub scope: Vec<String>,
    pub start_byte: usize,
}

/// Call expression site with its syntactic owner when visited inside a callable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallSiteMarker {
    /// Syntactic callee spelling, not a resolved identity. The structural marker
    /// producer replaces spellings over 1024 UTF-8 bytes with
    /// `<oversized-callee:blake3:...>`; inspect the source span for exact text.
    /// Such digest labels must remain unresolved by lexical name matching.
    pub callee: String,
    pub start_byte: usize,
    pub end_byte: usize,
    /// Captured from the callable stack during the AST walk. `None` means the
    /// call is file/module scoped, not that a later span search failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<DefinitionSiteRef>,
    /// Exact syntax sites. Absence means this producer did not inventory ports.
    /// Positions are source argument ordinals, never resolved parameter indexes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<CallSitePorts>,
}

/// Half-open UTF-8 byte range in the containing snapshot file, not a LogicId.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SiteSpan {
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentSiteKind {
    Positional,
    Named,
    Spread,
    KeywordSpread,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArgumentSite {
    /// Zero-based ordinal in the source argument list; spreads consume one site.
    pub position: usize,
    /// Source keyword label, when present. Not a resolved parameter binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: ArgumentSiteKind,
    /// Whole argument syntax, including labels, ref/out or spread operators.
    pub span: SiteSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SiteCompleteness {
    Complete,
    Partial,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SiteLimitation {
    InvalidSyntax,
    UnsupportedShape,
    ArgumentBudget,
    OversizedName,
}

/// Syntax associations only. The result is the call expression itself, including
/// void/discarded calls; it does not assert a returned value, assignment or flow.
/// A receiver is the syntactic selector base (possibly a type/namespace).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallSitePorts {
    pub callee: SiteSpan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<SiteSpan>,
    pub arguments: Vec<ArgumentSite>,
    pub result: SiteSpan,
    pub completeness: SiteCompleteness,
    pub limitations: Vec<SiteLimitation>,
}

impl CallSiteMarker {
    /// Validate syntax containment and budgets before durable publication.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.start_byte >= self.end_byte || self.callee.is_empty() || self.callee.len() > 1100 {
            return Err("invalid callsite span or callee");
        }
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| owner.name.is_empty() || owner.start_byte > self.start_byte)
        {
            return Err("invalid callsite owner");
        }
        let Some(ports) = &self.ports else {
            return Ok(());
        };
        let contained = |s: SiteSpan| {
            s.start_byte < s.end_byte
                && self.start_byte <= s.start_byte
                && s.end_byte <= self.end_byte
        };
        if !contained(ports.callee)
            || ports.receiver.is_some_and(|s| {
                !contained(s)
                    || s.start_byte < ports.callee.start_byte
                    || s.end_byte > ports.callee.end_byte
            })
            || ports.result
                != (SiteSpan {
                    start_byte: self.start_byte,
                    end_byte: self.end_byte,
                })
        {
            return Err("callsite port outside call expression");
        }
        if (ports.completeness == SiteCompleteness::Complete) != ports.limitations.is_empty() {
            return Err("site completeness disagrees with limitations");
        }
        if ports.arguments.len() > 256 {
            return Err("callsite argument budget exceeded");
        }
        let mut end = ports.callee.end_byte;
        for (position, argument) in ports.arguments.iter().enumerate() {
            if argument.position != position
                || !contained(argument.span)
                || argument.span.start_byte < end
            {
                return Err("invalid callsite argument order or span");
            }
            if argument
                .name
                .as_ref()
                .is_some_and(|n| n.is_empty() || n.len() > 1024)
                || (argument.name.is_some() && argument.kind != ArgumentSiteKind::Named)
            {
                return Err("invalid callsite argument name");
            }
            end = argument.span.end_byte;
        }
        Ok(())
    }
}

/// Durable snapshot-local syntax observation behind aggregate call relations.
/// Its ID addresses evidence, not a structural definition or value/taint identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallSiteRecord {
    pub site_id: String,
    pub snapshot: crate::SourceSnapshot,
    pub path: String,
    pub content_digest: crate::ContentDigest,
    pub marker: CallSiteMarker,
}

impl CallSiteRecord {
    pub const METHOD: &'static str = "entmoot.syntax.call_ports.v1";

    pub fn derive_id(&self) -> String {
        crate::hash_parts(&[
            Self::METHOD,
            &self.snapshot.repository_id,
            &self.snapshot.revision,
            self.snapshot.snapshot_digest.as_deref().unwrap_or(""),
            self.snapshot.estate_id.as_deref().unwrap_or(""),
            &format!("{:?}", self.snapshot.source_state),
            &self.path,
            self.content_digest.as_str(),
            &self.marker.start_byte.to_string(),
            &self.marker.end_byte.to_string(),
            &self.marker.callee,
        ])
    }

    pub fn validate(&self) -> Result<(), String> {
        self.snapshot.validate().map_err(|e| e.to_string())?;
        if crate::normalize_repo_path(&self.path).map_err(|e| e.to_string())? != self.path {
            return Err("callsite path must be canonical".into());
        }
        crate::ContentDigest::parse(self.content_digest.as_str()).map_err(|e| e.to_string())?;
        self.marker.validate().map_err(str::to_owned)?;
        if self.site_id != self.derive_id() {
            return Err("callsite identity mismatch".into());
        }
        Ok(())
    }
}

/// All markers collected from a single file during one AST walk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileMarkers {
    /// Source-declared boundary objects attached to inventory occurrences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_contracts: Vec<crate::CodeContract>,
    pub imports: Vec<ImportMarker>,
    pub definitions: Vec<DefMarker>,
    /// Non-body declarations and declaration-to-callable binding sites.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declarations: Vec<DeclarationMarker>,
    pub bindings: Vec<BindingMarker>,
    pub control_sites: Vec<ControlSite>,
    pub call_sites: Vec<CallSiteMarker>,
}

impl FileMarkers {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod initializer_value_tests {
    use super::*;

    fn syntax(text: &str, value: InitializerValue) -> DeclarationSyntax {
        DeclarationSyntax {
            method: DeclarationSyntax::METHOD.into(),
            name_span: SiteSpan {
                start_byte: 0,
                end_byte: 1,
            },
            initializer: Some(InitializerSyntax {
                span: SiteSpan {
                    start_byte: 2,
                    end_byte: 2 + text.len(),
                },
                content_digest: crate::ContentDigest::from_trusted(crate::blake3_hex(
                    text.as_bytes(),
                )),
                value,
            }),
        }
    }

    #[test]
    fn tagged_initializer_preserves_unevaluated_and_derives_decimal_from_text() {
        for (text, canonical) in [("- 1", "-1"), ("+7", "7"), ("0", "0")] {
            let value = syntax(text, InitializerValue::DecimalInteger { text: text.into() });
            value
                .validate(SiteSpan {
                    start_byte: 0,
                    end_byte: 2 + text.len(),
                })
                .unwrap();
            let initializer = value.initializer.as_ref().unwrap();
            assert_eq!(initializer.integer().as_deref(), Some(canonical));
            let encoded = serde_json::to_value(initializer).unwrap();
            assert!(encoded.get("integer").is_none());
            assert_eq!(
                serde_json::from_value::<InitializerSyntax>(encoded).unwrap(),
                *initializer
            );
        }
        let unknown = syntax("f()", InitializerValue::Unevaluated {});
        unknown
            .validate(SiteSpan {
                start_byte: 0,
                end_byte: 5,
            })
            .unwrap();
        assert_eq!(unknown.initializer.unwrap().integer(), None);
    }

    #[test]
    fn tagged_initializer_rejects_legacy_partial_contradictory_and_forged_values() {
        for value in [
            serde_json::json!({"kind":"decimal_integer"}),
            serde_json::json!({"kind":"decimal_integer","text":"1","integer":"2"}),
            serde_json::json!({"kind":"unevaluated","text":"1"}),
            serde_json::json!({"literal_text":"1","integer":"1"}),
            serde_json::Value::Null,
        ] {
            assert!(
                serde_json::from_value::<InitializerValue>(value.clone()).is_err(),
                "{value}"
            );
        }
        for text in ["0x10", "01", "-0", "1 + 2", ""] {
            let value = syntax(text, InitializerValue::DecimalInteger { text: text.into() });
            assert!(
                value
                    .validate(SiteSpan {
                        start_byte: 0,
                        end_byte: 2 + text.len()
                    })
                    .is_err(),
                "{text}"
            );
        }
        let forged = syntax("1", InitializerValue::DecimalInteger { text: "2".into() });
        assert!(forged
            .validate(SiteSpan {
                start_byte: 0,
                end_byte: 3
            })
            .is_err());
        let legacy = serde_json::json!({
            "span":{"start_byte":2,"end_byte":3},
            "content_digest":crate::blake3_hex(b"1"),
            "literal_text":"1", "integer":"1"
        });
        assert!(serde_json::from_value::<InitializerSyntax>(legacy).is_err());
    }
}
