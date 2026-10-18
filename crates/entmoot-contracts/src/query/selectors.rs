//! Composable selection over attributable inventory observations.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Inventory vocabulary shared by extraction projections and public selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InventoryKind {
    /// Repository root.
    Repo,
    /// Source file.
    File,
    /// Referenced module.
    ModuleStub,
    /// Declared callable.
    Callable,
    /// Named declaration.
    Declaration,
    /// Inventoried call site.
    CallStub,
    /// Control-flow syntax site.
    ControlSite,
    /// Attributed resource.
    Resource,
}

impl InventoryKind {
    /// Stable wire spelling; no display/debug formatting enters selectors.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repo => "repo",
            Self::File => "file",
            Self::ModuleStub => "module_stub",
            Self::Callable => "callable",
            Self::Declaration => "declaration",
            Self::CallStub => "call_stub",
            Self::ControlSite => "control_site",
            Self::Resource => "resource",
        }
    }
}

/// Character comparison policy. Lowercase uses Rust Unicode lowercase mapping;
/// it is not locale-sensitive collation or Unicode normalization.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextCase {
    /// Compare the supplied spelling.
    #[default]
    Sensitive,
    /// Compare Unicode lowercase mappings.
    Lowercase,
}

/// Explicit textual matching. Fuzzy matching never establishes identity or type equivalence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextMatch {
    /// Whole-field equality.
    Exact {
        /// Nonempty literal, at most 1024 UTF-8 bytes.
        value: String,
        /// Character comparison policy.
        #[serde(default)]
        case: TextCase,
    },
    /// Contiguous literal substring.
    Contains {
        /// Nonempty literal, at most 1024 UTF-8 bytes.
        value: String,
        /// Character comparison policy.
        #[serde(default)]
        case: TextCase,
    },
    /// Whole-field Unicode scalar edit distance, with unit insertion/deletion/substitution.
    Levenshtein {
        /// Nonempty literal, at most 1024 UTF-8 bytes.
        value: String,
        /// Maximum distance, 0..=8. Never an implicit fallback from exact/contains.
        max_distance: usize,
        /// Character comparison policy.
        #[serde(default)]
        case: TextCase,
    },
}

impl TextMatch {
    /// Validate bounded textual selection independently of a query backend.
    ///
    /// # Errors
    /// Returns an error for empty/oversized text or an unsupported distance cutoff.
    pub fn validate(&self) -> Result<(), String> {
        let value = match self {
            Self::Exact { value, .. } | Self::Contains { value, .. } => value,
            Self::Levenshtein {
                value,
                max_distance,
                ..
            } => {
                if *max_distance > 8 {
                    return Err("Levenshtein max_distance must be 0..=8".into());
                }
                value
            }
        };
        if value.is_empty() || value.len() > 1024 {
            return Err("text selector must contain 1..=1024 UTF-8 bytes".into());
        }
        Ok(())
    }
}

/// Repository-relative path selection, independent of host path conventions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "mode",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PathSelector {
    /// One exact file or inventory path.
    Exact(String),
    /// Exact path and descendants at slash boundaries.
    Subtree(String),
    /// Case-sensitive globset syntax: '*' stays within a segment; '**' crosses segments.
    Glob(String),
}

impl PathSelector {
    /// Normalize repository coordinates; backend compilation validates glob syntax.
    ///
    /// # Errors
    /// Returns an error for oversized, absolute or traversing paths.
    pub fn normalized_value(&self) -> Result<String, String> {
        let (Self::Exact(value) | Self::Subtree(value) | Self::Glob(value)) = self;
        if value.len() > 4096 {
            return Err("path selector exceeds 4096 bytes".into());
        }
        crate::normalize_repo_path(value).map_err(|error| error.to_string())
    }
}

/// Constraints on one written parameter. All fields bind to the same parameter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ParameterSelector {
    /// Zero-based index in the written parameter vector, including a written receiver.
    pub position: Option<usize>,
    /// Written name.
    pub name: Option<TextMatch>,
    /// Language-neutral parameter category.
    pub kind: Option<crate::ParameterKind>,
    /// Explicit written type syntax only; unspecified/unsupported types are unknown.
    pub declared_type_syntax: Option<TextMatch>,
}

/// Selection over the published source-declared callable shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CallableSelector {
    /// Each requirement must match at least one parameter. Requirements may reuse
    /// a parameter; use explicit positions to require distinct parameters.
    pub parameters: Vec<ParameterSelector>,
    /// Written result type. This does not perform inference or alias resolution.
    pub result_type_syntax: Option<TextMatch>,
}

/// Conjunction of inventory predicates; empty lists do not constrain their field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SubjectSelector {
    /// Admitted inventory categories.
    pub kinds: Vec<InventoryKind>,
    /// Exact published language names; missing language evidence is unknown.
    pub languages: Vec<String>,
    /// Callable/declaration name from its contract, otherwise the inventory label.
    pub name: Option<TextMatch>,
    /// Full qualified display name, when reported.
    pub qualified_name: Option<TextMatch>,
    /// Repository-relative path predicate.
    pub path: Option<PathSelector>,
    /// Constraints on callable declarations.
    pub callable: Option<CallableSelector>,
}

impl SubjectSelector {
    /// Validate compositional and work limits before inspecting inventory.
    ///
    /// # Errors
    /// Returns an error for invalid text/path predicates or excessive requirements.
    pub fn validate(&self) -> Result<(), String> {
        if self.languages.len() > 32 || self.languages.iter().any(|s| s.is_empty() || s.len() > 64)
        {
            return Err("languages requires at most 32 nonempty names of at most 64 bytes".into());
        }
        for text in [&self.name, &self.qualified_name].into_iter().flatten() {
            text.validate()?;
        }
        if let Some(path) = &self.path {
            path.normalized_value()?;
        }
        if let Some(callable) = &self.callable {
            if callable.parameters.len() > 32 {
                return Err("at most 32 parameter requirements are supported".into());
            }
            for parameter in &callable.parameters {
                for text in [&parameter.name, &parameter.declared_type_syntax]
                    .into_iter()
                    .flatten()
                {
                    text.validate()?;
                }
            }
            if let Some(result) = &callable.result_type_syntax {
                result.validate()?;
            }
        }
        Ok(())
    }
}

/// Why an inventory candidate satisfied an explicit selector.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectionMatch {
    /// Each parameter requirement's matching zero-based positions, in request order.
    pub parameter_positions: Vec<Vec<usize>>,
}

/// Selection accounting over examined inventory, independently of output paging.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectionReceipt {
    /// Candidates evaluated before paging.
    pub examined: usize,
    /// Evaluated candidates missing facts required by their selector.
    pub unknown: usize,
    /// Exact admitted candidate count before paging.
    pub matched: usize,
}

/// Exclusions applied while discovering relations, never after selecting witnesses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TraversalPrune {
    /// Exact inventory references; missing or ambiguous references are rejected.
    pub subjects: Vec<String>,
    /// Exact represented relation occurrence IDs.
    pub relation_edges: Vec<String>,
    /// Published relation occurrence keys, including source callsite keys.
    pub occurrences: Vec<String>,
    /// Inventory subjects matching these predicates cannot be crossed.
    /// Missing selector facts never establish a match.
    pub matching_subjects: Option<Box<SubjectSelector>>,
}
