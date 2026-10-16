//! Logic addresses — declarative envelopes for definition sites.
//!
//! Vocabulary: see `docs/architecture/ARCHITECTURE.md` in the repo root.
//!
//! - **Logic address** (`logic_address`): canonical `logic|repository|lang|…` string
//! - **Address id** (`address_id`): BLAKE3 hex of that string
//!
//! Structural hashes (body/signature/file) live on [`CallableHash`](crate::granular::CallableHash);
//! they answer "did logic change?", not "where is this definition?".

use crate::hash::hash_address_envelope;

/// Empty optional field sentinel in the canonical form.
pub const EMPTY: &str = "-";

/// Fixed kind vocabulary shared across languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicKind {
    Module,
    Class,
    Struct,
    Enum,
    Interface,
    Trait,
    /// A named type whose syntax is not a class, struct, enum or interface.
    Type,
    /// An explicit syntax-level alias; resolution never changes this kind.
    TypeAlias,
    Impl,
    Function,
    Method,
    Constructor,
    Accessor,
    Macro,
    Test,
    AssignedCallable,
    Binding,
}

impl LogicKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Type => "type",
            Self::TypeAlias => "type_alias",
            Self::Impl => "impl",
            Self::Function => "function",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Accessor => "accessor",
            Self::Macro => "macro",
            Self::Test => "test",
            Self::AssignedCallable => "assigned_callable",
            Self::Binding => "binding",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "module" => Some(Self::Module),
            "class" => Some(Self::Class),
            "struct" => Some(Self::Struct),
            "enum" => Some(Self::Enum),
            "interface" => Some(Self::Interface),
            "trait" => Some(Self::Trait),
            "type" => Some(Self::Type),
            "type_alias" => Some(Self::TypeAlias),
            "impl" => Some(Self::Impl),
            "function" => Some(Self::Function),
            "method" => Some(Self::Method),
            "constructor" => Some(Self::Constructor),
            "accessor" => Some(Self::Accessor),
            "macro" => Some(Self::Macro),
            "test" => Some(Self::Test),
            "assigned_callable" => Some(Self::AssignedCallable),
            "binding" => Some(Self::Binding),
            _ => None,
        }
    }

    /// Map visitor node_type strings to a kind.
    ///
    /// When `inside_type` is true (directly inside a type-like body, not merely
    /// a non-empty FQN scope), bare `function` is treated as `method`
    /// (Python/JS methods declared with function nodes).
    pub fn from_node_type(node_type: &str, inside_type: bool) -> Self {
        match node_type {
            "method" => Self::Method,
            "constructor" | "constructor_declaration" => Self::Constructor,
            "class" | "class_definition" | "class_declaration" => Self::Class,
            "struct" | "struct_item" => Self::Struct,
            "enum" | "enum_item" | "enum_declaration" => Self::Enum,
            "interface" | "interface_declaration" => Self::Interface,
            "trait" | "trait_item" => Self::Trait,
            "type" => Self::Type,
            "type_alias" | "type_alias_declaration" => Self::TypeAlias,
            "impl" | "impl_item" => Self::Impl,
            "accessor" | "get" | "set" => Self::Accessor,
            "macro" | "macro_definition" => Self::Macro,
            "test" => Self::Test,
            "module" => Self::Module,
            "function"
            | "function_definition"
            | "function_declaration"
            | "arrow_function"
            | "function_expression"
            | "local_function_statement" => {
                if inside_type {
                    Self::Method
                } else {
                    Self::Function
                }
            }
            _ => {
                if inside_type {
                    Self::Method
                } else {
                    Self::Function
                }
            }
        }
    }
}

impl std::fmt::Display for LogicKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Repository / language / path context for assigning logic addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressMeta {
    pub repository: String,
    pub lang: String,
    pub path: String,
}

impl AddressMeta {
    pub fn new(lang: impl Into<String>) -> Self {
        Self {
            repository: EMPTY.to_string(),
            lang: normalize_lang(lang.into()),
            path: EMPTY.to_string(),
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = normalize_path(&path.into());
        self
    }

    pub fn with_repository(mut self, repository: impl Into<String>) -> Self {
        let r = repository.into();
        self.repository = if r.is_empty() { EMPTY.to_string() } else { r };
        self
    }
}

/// Language-agnostic logic address for a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicAddress {
    pub repository: String,
    pub lang: String,
    pub path: String,
    /// Declared container names, outer → inner (not the symbol itself).
    pub scope: Vec<String>,
    pub symbol: String,
    pub kind: LogicKind,
    pub start: Option<usize>,
    pub end: Option<usize>,
}

impl LogicAddress {
    pub fn new(
        meta: &AddressMeta,
        scope: Vec<String>,
        symbol: impl Into<String>,
        kind: LogicKind,
        start: Option<usize>,
        end: Option<usize>,
    ) -> Self {
        Self {
            repository: field_or_empty(&meta.repository),
            lang: normalize_lang(&meta.lang),
            path: normalize_path(&meta.path),
            scope: scope
                .into_iter()
                .map(|s| sanitize_segment(&s))
                .filter(|s| !s.is_empty() && s != EMPTY)
                .collect(),
            symbol: sanitize_segment(&symbol.into()),
            kind,
            start,
            end,
        }
    }

    /// Canonical delimited form used for `address_id`.
    pub fn canonical(&self) -> String {
        format!(
            "logic|{}|{}|{}|{}|{}|{}|{}|{}",
            self.repository,
            self.lang,
            self.path,
            scope_string(&self.scope),
            if self.symbol.is_empty() { EMPTY } else { &self.symbol },
            self.kind.as_str(),
            opt_usize(self.start),
            opt_usize(self.end),
        )
    }

    /// Without byte span — used as the preferred address when unique.
    pub fn without_span(&self) -> Self {
        let mut a = self.clone();
        a.start = None;
        a.end = None;
        a
    }

    /// With byte span for disambiguation.
    pub fn with_span(mut self, start: usize, end: usize) -> Self {
        self.start = Some(start);
        self.end = Some(end);
        self
    }

    /// BLAKE3 hex of [`Self::canonical`] (no `blake3:` prefix).
    pub fn address_id(&self) -> String {
        hash_address_envelope(&self.canonical())
    }

    /// Optional URI-style display form (not used for hashing).
    pub fn display_uri(&self) -> String {
        let scope_path = if self.scope.is_empty() {
            String::new()
        } else {
            format!("/{}/", self.scope.join("/"))
        };
        let path = if self.path == EMPTY {
            String::new()
        } else {
            format!("/{}", self.path.trim_start_matches('/'))
        };
        let repository = if self.repository == EMPTY {
            "_"
        } else {
            &self.repository
        };
        let mut uri = format!(
            "logic://{repository}{path}?lang={}#{scope_path}{}@{}",
            self.lang,
            self.symbol,
            self.kind.as_str()
        );
        if let (Some(s), Some(e)) = (self.start, self.end) {
            uri.push_str(&format!(";start={s};end={e}"));
        }
        uri
    }
}

fn scope_string(scope: &[String]) -> String {
    if scope.is_empty() {
        EMPTY.to_string()
    } else {
        scope.join("/")
    }
}

fn opt_usize(v: Option<usize>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| EMPTY.to_string())
}

fn field_or_empty(s: &str) -> String {
    if s.is_empty() || s == EMPTY {
        EMPTY.to_string()
    } else {
        s.to_string()
    }
}

fn normalize_lang(lang: impl AsRef<str>) -> String {
    let l = lang.as_ref().trim().to_lowercase();
    if l.is_empty() {
        EMPTY.to_string()
    } else {
        l
    }
}

/// Repo-relative POSIX path; empty → `-`.
fn normalize_path(path: &str) -> String {
    let p = path.trim().replace('\\', "/");
    let p = p.trim_start_matches("./");
    if p.is_empty() || p == EMPTY {
        return EMPTY.to_string();
    }
    let mut parts = Vec::new();
    for part in p.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            let _ = parts.pop();
            continue;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        EMPTY.to_string()
    } else {
        parts.join("/")
    }
}

/// Scope/symbol segments must not contain `|` or `/`.
fn sanitize_segment(s: &str) -> String {
    s.trim().replace(['|', '/'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_includes_lang_as_field() {
        let meta = AddressMeta::new("Python")
            .with_path("src\\auth\\service.py")
            .with_repository("acme/app");
        let addr = LogicAddress::new(
            &meta,
            vec!["AuthService".into()],
            "verify",
            LogicKind::Method,
            Some(10),
            Some(40),
        );
        assert_eq!(
            addr.canonical(),
            "logic|acme/app|python|src/auth/service.py|AuthService|verify|method|10|40"
        );
        assert!(!addr.address_id().is_empty());
        assert_eq!(addr.address_id().len(), 64);
    }

    #[test]
    fn empty_fields_use_sentinel() {
        let meta = AddressMeta::new("go");
        let addr = LogicAddress::new(&meta, vec![], "Handle", LogicKind::Function, None, None);
        assert_eq!(addr.canonical(), "logic|-|go|-|-|Handle|function|-|-");
    }

    #[test]
    fn function_inside_type_becomes_method() {
        assert_eq!(LogicKind::from_node_type("function", true), LogicKind::Method);
        assert_eq!(LogicKind::from_node_type("function", false), LogicKind::Function);
    }

    #[test]
    fn without_span_changes_address_id() {
        let meta = AddressMeta::new("python").with_path("a.py");
        let with = LogicAddress::new(&meta, vec![], "f", LogicKind::Function, Some(0), Some(10));
        let without = with.without_span();
        assert_ne!(with.address_id(), without.address_id());
        assert!(without.canonical().ends_with("|-|-"));
    }

    #[test]
    fn display_uri_readable() {
        let meta = AddressMeta::new("java")
            .with_path("com/Acme.java")
            .with_repository("github.com/acme/x");
        let addr = LogicAddress::new(&meta, vec!["Acme".into()], "run", LogicKind::Method, None, None);
        assert_eq!(
            addr.display_uri(),
            "logic://github.com/acme/x/com/Acme.java?lang=java#/Acme/run@method"
        );
    }
}
