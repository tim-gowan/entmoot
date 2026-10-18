//! Compile selectors once and evaluate published observations before hydration/paging.
use entmoot_contracts::query::{
    CallableSelector, ParameterSelector, PathSelector, SelectionMatch, SubjectSelector, TextCase,
    TextMatch,
};
use entmoot_contracts::{DeclaredContract, DeclaredParameter, DeclaredType};
use rapidfuzz::distance::levenshtein;

use super::QueryError;
use crate::TopologyNode;

/// Three-valued conjunction: a definite mismatch dominates missing evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatchState {
    Yes,
    No,
    Unknown,
}
impl MatchState {
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            _ => Self::Yes,
        }
    }
}

pub(super) enum CompiledPath {
    LegacyPrefix(String),
    Exact(String),
    Subtree(String),
    Glob(globset::GlobMatcher),
}
impl CompiledPath {
    /// Explicit selectors take no hidden precedence over legacy fields.
    pub fn from_request(
        selector: Option<&PathSelector>,
        legacy: Option<&str>,
    ) -> Result<Option<Self>, QueryError> {
        if selector.is_some() && legacy.is_some() {
            return Err(QueryError::InvalidInput(
                "path and path_selector are mutually exclusive".into(),
            ));
        }
        match selector {
            Some(selector) => Self::new(selector).map(Some),
            None => legacy
                .map(|p| {
                    let value = p.trim().replace('\\', "/");
                    PathSelector::Subtree(value.clone())
                        .normalized_value()
                        .map_err(QueryError::InvalidInput)?;
                    // A trailing slash was meaningful to the legacy raw prefix.
                    Ok(Self::LegacyPrefix(value))
                })
                .transpose(),
        }
    }
    pub fn new(selector: &PathSelector) -> Result<Self, QueryError> {
        let value = selector
            .normalized_value()
            .map_err(QueryError::InvalidInput)?;
        Ok(match selector {
            PathSelector::Exact(_) => Self::Exact(value),
            PathSelector::Subtree(_) => Self::Subtree(value),
            PathSelector::Glob(_) => Self::Glob(
                globset::GlobBuilder::new(&value)
                    .literal_separator(true)
                    .backslash_escape(false)
                    .build()
                    .map_err(|e| QueryError::InvalidInput(format!("invalid path glob: {e}")))?
                    .compile_matcher(),
            ),
        })
    }
    pub fn matches(&self, path: &str) -> bool {
        match self {
            Self::LegacyPrefix(value) => path.starts_with(value),
            Self::Exact(value) => path == value,
            Self::Subtree(value) => {
                path == value
                    || path
                        .strip_prefix(value)
                        .is_some_and(|rest| rest.starts_with('/'))
            }
            Self::Glob(matcher) => matcher.is_match(path),
        }
    }
}

struct CompiledText {
    value: String,
    case: TextCase,
    mode: TextMode,
}
enum TextMode {
    Exact,
    Contains,
    Levenshtein(usize),
}
impl CompiledText {
    fn new(selector: &TextMatch) -> Result<Self, QueryError> {
        selector.validate().map_err(QueryError::InvalidInput)?;
        let (value, case, mode) = match selector {
            TextMatch::Exact { value, case } => (value, *case, TextMode::Exact),
            TextMatch::Contains { value, case } => (value, *case, TextMode::Contains),
            TextMatch::Levenshtein {
                value,
                case,
                max_distance,
            } => (value, *case, TextMode::Levenshtein(*max_distance)),
        };
        Ok(Self {
            value: match case {
                TextCase::Sensitive => value.clone(),
                TextCase::Lowercase => value.to_lowercase(),
            },
            case,
            mode,
        })
    }
    fn matches(&self, value: Option<&str>) -> MatchState {
        let Some(value) = value else {
            return MatchState::Unknown;
        };
        let lowered;
        let value = if self.case == TextCase::Lowercase {
            lowered = value.to_lowercase();
            lowered.as_str()
        } else {
            value
        };
        let matched = match self.mode {
            TextMode::Exact => value == self.value,
            TextMode::Contains => value.contains(&self.value),
            TextMode::Levenshtein(max) => levenshtein::distance_with_args(
                self.value.chars(),
                value.chars(),
                &levenshtein::Args::default().score_cutoff(max),
            )
            .is_some(),
        };
        if matched {
            MatchState::Yes
        } else {
            MatchState::No
        }
    }
    fn declared_type(&self, value: &DeclaredType) -> MatchState {
        self.matches(match value {
            DeclaredType::Explicit { syntax, .. } => Some(syntax),
            DeclaredType::Unspecified | DeclaredType::Unsupported { .. } => None,
        })
    }
}

struct Parameter {
    position: Option<usize>,
    kind: Option<entmoot_contracts::ParameterKind>,
    name: Option<CompiledText>,
    ty: Option<CompiledText>,
}
impl Parameter {
    fn new(value: &ParameterSelector) -> Result<Self, QueryError> {
        Ok(Self {
            position: value.position,
            kind: value.kind,
            name: value.name.as_ref().map(CompiledText::new).transpose()?,
            ty: value
                .declared_type_syntax
                .as_ref()
                .map(CompiledText::new)
                .transpose()?,
        })
    }
    fn matches(&self, position: usize, parameter: &DeclaredParameter) -> MatchState {
        if self.position.is_some_and(|p| p != position)
            || self.kind.is_some_and(|k| k != parameter.kind)
        {
            return MatchState::No;
        }
        let name = self
            .name
            .as_ref()
            .map_or(MatchState::Yes, |m| m.matches(parameter.name.as_deref()));
        name.and(self.ty.as_ref().map_or(MatchState::Yes, |m| {
            m.declared_type(&parameter.declared_type)
        }))
    }
}
struct Callable {
    parameters: Vec<Parameter>,
    result: Option<CompiledText>,
}
impl Callable {
    fn new(value: &CallableSelector) -> Result<Self, QueryError> {
        Ok(Self {
            parameters: value
                .parameters
                .iter()
                .map(Parameter::new)
                .collect::<Result<_, _>>()?,
            result: value
                .result_type_syntax
                .as_ref()
                .map(CompiledText::new)
                .transpose()?,
        })
    }
}

pub(super) struct CompiledSelector {
    selector: SubjectSelector,
    name: Option<CompiledText>,
    qualified_name: Option<CompiledText>,
    path: Option<CompiledPath>,
    callable: Option<Callable>,
}
impl CompiledSelector {
    pub fn new(selector: &SubjectSelector) -> Result<Self, QueryError> {
        selector.validate().map_err(QueryError::InvalidInput)?;
        Ok(Self {
            selector: selector.clone(),
            name: selector.name.as_ref().map(CompiledText::new).transpose()?,
            qualified_name: selector
                .qualified_name
                .as_ref()
                .map(CompiledText::new)
                .transpose()?,
            path: selector.path.as_ref().map(CompiledPath::new).transpose()?,
            callable: selector.callable.as_ref().map(Callable::new).transpose()?,
        })
    }
    pub fn matches(&self, node: &TopologyNode) -> (MatchState, SelectionMatch) {
        let mut explanation = SelectionMatch::default();
        if !self.selector.kinds.is_empty() && !self.selector.kinds.contains(&node.kind) {
            return (MatchState::No, explanation);
        }
        let mut state = MatchState::Yes;
        if let Some(path) = &self.path {
            state = state.and(node.path.as_deref().map_or(MatchState::Unknown, |p| {
                if path.matches(p) {
                    MatchState::Yes
                } else {
                    MatchState::No
                }
            }));
        }
        if !self.selector.languages.is_empty() {
            state = state.and(
                node.code_contract
                    .as_ref()
                    .map_or(MatchState::Unknown, |c| {
                        if self.selector.languages.contains(&c.language) {
                            MatchState::Yes
                        } else {
                            MatchState::No
                        }
                    }),
            );
        }
        if let Some(name) = &self.name {
            state = state.and(name.matches(Some(
                node.code_contract.as_ref().map_or(&node.label, |c| &c.name),
            )));
        }
        if let Some(name) = &self.qualified_name {
            state = state.and(name.matches(node.fqn.as_deref()));
        }
        if let Some(callable) = &self.callable {
            if node.kind != crate::NodeKind::Callable {
                return (MatchState::No, explanation);
            }
            let Some(contract) = &node.code_contract else {
                return (state.and(MatchState::Unknown), explanation);
            };
            let DeclaredContract::Callable { parameters, result } = &contract.shape else {
                return (state.and(MatchState::Unknown), explanation);
            };
            for requirement in &callable.parameters {
                let mut positions = Vec::new();
                let mut unknown = !contract.limitations.is_empty();
                for (position, parameter) in parameters.iter().enumerate() {
                    match requirement.matches(position, parameter) {
                        MatchState::Yes => positions.push(position),
                        MatchState::Unknown => unknown = true,
                        MatchState::No => {}
                    }
                }
                state = state.and(if !positions.is_empty() {
                    MatchState::Yes
                } else if unknown {
                    MatchState::Unknown
                } else {
                    MatchState::No
                });
                explanation.parameter_positions.push(positions);
            }
            if let Some(matcher) = &callable.result {
                state = state.and(matcher.declared_type(result));
            }
        }
        (state, explanation)
    }
}
