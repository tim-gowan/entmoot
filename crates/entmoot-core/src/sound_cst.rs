//! Sound structural CST serialization (policy v1 / ADR-0024).
//!
//! Retains every non-layout token. Prunes only whitespace/layout and
//! non-directive comments. Directive comments are a per-language allowlist;
//! unknown comment shapes are retained.

use std::borrow::Cow;
use std::collections::HashMap;

use tree_sitter::Node;

/// Per-language comment and directive rules for sound structural hashing.
#[derive(Debug, Clone)]
pub struct SoundStructuralProfile {
    /// Tree-sitter node kinds treated as comments.
    pub comment_kinds: &'static [&'static str],
    /// Prefixes (after leading comment markers/whitespace) that keep a comment.
    pub directive_prefixes: &'static [&'static str],
}

impl SoundStructuralProfile {
    pub fn for_language(language: &str) -> Self {
        match language.to_lowercase().as_str() {
            "python" => Self {
                comment_kinds: &["comment"],
                directive_prefixes: &["type:", " noqa", " pragma:", " coding:", " -*-"],
            },
            "javascript" | "typescript" | "tsx" => Self {
                comment_kinds: &["comment"],
                directive_prefixes: &[
                    "@ts-",
                    "@jsx",
                    "@__PURE__",
                    "#__PURE__",
                    "eslint-",
                    "prettier-",
                    "istanbul ",
                    "c8 ",
                    "webpack",
                    "vite-",
                ],
            },
            "go" => Self {
                comment_kinds: &["comment"],
                directive_prefixes: &[
                    "go:build",
                    "go:generate",
                    "go:embed",
                    "go:linkname",
                    "go:noinline",
                    "go:nosplit",
                    "go:norace",
                    "go:uintptrescapes",
                    "go:noescape",
                    "go:mustinline",
                    "go:systemstack",
                    "go:nowritebarrier",
                    "go:nowritebarrierrec",
                    "go:yeswritebarrierrec",
                    "+build",
                    "export ",
                ],
            },
            // Rust doc comments are `#[doc]` attributes: semantic, retained.
            "rust" => Self {
                comment_kinds: &["line_comment", "block_comment"],
                directive_prefixes: &["///", "//!", "/**", "/*!"],
            },
            // No comment-borne compiler directives; preprocessor/annotations are not comments.
            "java" => Self {
                comment_kinds: &["line_comment", "block_comment"],
                directive_prefixes: &[],
            },
            "c" | "cpp" => Self {
                comment_kinds: &["comment"],
                directive_prefixes: &["clang", "NOLINT", "IWYU", "coverity", "@", "fallthrough"],
            },
            "csharp" => Self {
                comment_kinds: &["comment"],
                directive_prefixes: &[],
            },
            // Unknown grammars: nothing is classified as a comment, so every node is retained.
            _ => Self {
                comment_kinds: &[],
                directive_prefixes: &[],
            },
        }
    }

    fn is_comment_kind(&self, kind: &str) -> bool {
        self.comment_kinds.contains(&kind)
    }

    fn is_directive_comment(&self, text: &str) -> bool {
        let trimmed = text
            .trim_start()
            .trim_start_matches(['/', '#', '*', '-', ' ', '\t'])
            .trim_start();
        self.directive_prefixes
            .iter()
            .any(|prefix| text.starts_with(prefix) || trimmed.starts_with(prefix) || text.contains(prefix))
    }
}

/// Canonical retained IR, allocated only for signatures or explicit inspection.
pub fn serialize_sound_span(source: &[u8], node: Node<'_>, profile: &SoundStructuralProfile) -> String {
    let mut ir = String::new();
    write_node(source, node, None, profile, &mut |part| ir.push_str(part));
    ir
}

pub(crate) fn retained(source: &[u8], node: Node<'_>, profile: &SoundStructuralProfile) -> bool {
    if !node.is_named() {
        return !node_text(source, node).chars().all(char::is_whitespace);
    }
    !profile.is_comment_kind(node.kind()) || profile.is_directive_comment(&node_text(source, node))
}

// Stream the exact v1 token sequence. No per-node strings or child-string lists.
fn write_node(
    source: &[u8],
    node: Node<'_>,
    omitted_name: Option<Node<'_>>,
    profile: &SoundStructuralProfile,
    emit: &mut impl FnMut(&str),
) {
    if Some(node) == omitted_name {
        emit("(declared_name:");
        emit(node.kind());
        emit(")");
        return;
    }
    if !retained(source, node, profile) {
        return;
    }
    emit("(");
    if !node.is_named() {
        emit("tok:");
        emit(&node_text(source, node));
    } else if profile.is_comment_kind(node.kind()) {
        emit("comment:");
        emit(&node_text(source, node));
    } else {
        emit(node.kind());
        if node.child_count() == 0 {
            let text = node_text(source, node);
            if !text.is_empty() {
                emit(":");
                emit(&text);
            }
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if retained(source, child, profile) {
                    emit(" ");
                    write_node(source, child, omitted_name, profile, emit);
                }
            }
        }
    }
    emit(")");
}

fn node_text<'a>(source: &'a [u8], node: Node<'_>) -> Cow<'a, str> {
    source
        .get(node.start_byte()..node.end_byte())
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
}

/// One bounded lookup index for all inventoried callable spans in this file.
/// The original reverse-child DFS order preserves the innermost exact match.
pub(crate) struct CallableNodes<'tree> {
    nodes: HashMap<(usize, usize), Option<Node<'tree>>>,
}

impl<'tree> CallableNodes<'tree> {
    pub(crate) fn new(root: Node<'tree>, spans: impl Iterator<Item = (usize, usize)>) -> Self {
        let mut nodes: HashMap<_, _> = spans.map(|span| (span, None)).collect();
        if !nodes.is_empty() {
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                if let Some(entry) = nodes.get_mut(&(node.start_byte(), node.end_byte())) {
                    *entry = Some(node);
                }
                let mut cursor = node.walk();
                stack.extend(node.children(&mut cursor));
            }
        }
        Self { nodes }
    }

    pub(crate) fn get(&self, start: usize, end: usize) -> Option<Node<'tree>> {
        self.nodes.get(&(start, end)).copied().flatten()
    }
}

/// Token-preserving boundary IR excluding the declaration name and body.
pub(crate) fn callable_signature(source: &[u8], node: Node<'_>, profile: &SoundStructuralProfile) -> String {
    let mut ir = format!("entmoot.callable_signature.v1({} ", node.kind());
    let mut cursor = node.walk();
    let mut first = true;
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if !matches!(cursor.field_name(), Some("name" | "body")) && retained(source, child, profile) {
                if !first {
                    ir.push(' ');
                }
                first = false;
                write_node(source, child, None, profile, &mut |part| ir.push_str(part));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    ir.push(')');
    ir
}

/// Retained declaration tokens and sorted imports; declared name is location,
/// not structure. An inventoried span must join the CST before reaching here.
pub(crate) fn hash_sound_callable(
    source: &[u8],
    node: Node<'_>,
    imports: &[String],
    profile: &SoundStructuralProfile,
) -> String {
    let mut hash = blake3::Hasher::new();
    let mut emit = |part: &str| {
        hash.update(part.as_bytes());
    };
    let c_name = (node.kind() == "function_definition" && node.child_by_field_name("name").is_none())
        .then(|| {
            node.child_by_field_name("declarator")
                .and_then(crate::languages::c_family::declarator_name)
        })
        .flatten();
    if let Some(name) = c_name {
        write_node(source, node, Some(name), profile, &mut emit);
    } else {
        emit("(");
        emit(node.kind());
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if cursor.field_name() != Some("name") && retained(source, child, profile) {
                    emit(" ");
                    write_node(source, child, None, profile, &mut emit);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        emit(")");
    }
    let mut imports: Vec<_> = imports.iter().map(String::as_str).collect();
    imports.sort_unstable();
    // The empty import set still contributes its leading separator.
    emit("|");
    for (i, import) in imports.into_iter().enumerate() {
        if i > 0 {
            emit("|");
        }
        emit(import);
    }
    format!("blake3:{}", hash.finalize().to_hex())
}

/// BLAKE3 of the full-file retained CST without allocating inspection output.
pub fn hash_sound_module(source: &[u8], root: Node<'_>, profile: &SoundStructuralProfile) -> String {
    hash_module(source, root, profile, None)
}

pub(crate) fn hash_module(
    source: &[u8],
    root: Node<'_>,
    profile: &SoundStructuralProfile,
    mut debug_ir: Option<&mut String>,
) -> String {
    let mut hash = blake3::Hasher::new();
    write_node(source, root, None, profile, &mut |part| {
        hash.update(part.as_bytes());
        if let Some(ir) = debug_ir.as_mut() {
            ir.push_str(part);
        }
    });
    format!("blake3:{}", hash.finalize().to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_python(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::PYTHON.into())
            .expect("python grammar");
        parser.parse(source, None).expect("parse")
    }

    #[test]
    fn layout_and_non_directive_comments_do_not_change_hash() {
        let a = "def f():\n    return 1\n";
        let b = "def f():\n\n    # cosmetic\n    return 1\n";
        let profile = SoundStructuralProfile::for_language("python");
        let tree_a = parse_python(a);
        let tree_b = parse_python(b);
        let ha = hash_sound_module(a.as_bytes(), tree_a.root_node(), &profile);
        let hb = hash_sound_module(b.as_bytes(), tree_b.root_node(), &profile);
        assert_eq!(ha, hb);
    }

    #[test]
    fn retained_literal_change_changes_hash() {
        let a = "def f():\n    return 1\n";
        let b = "def f():\n    return 2\n";
        let profile = SoundStructuralProfile::for_language("python");
        let tree_a = parse_python(a);
        let tree_b = parse_python(b);
        let ha = hash_sound_module(a.as_bytes(), tree_a.root_node(), &profile);
        let hb = hash_sound_module(b.as_bytes(), tree_b.root_node(), &profile);
        assert_ne!(ha, hb);
    }

    #[test]
    fn type_comment_directive_is_retained() {
        let a = "def f(x):\n    # type: (int) -> int\n    return x\n";
        let b = "def f(x):\n    # type: (str) -> str\n    return x\n";
        let profile = SoundStructuralProfile::for_language("python");
        let tree_a = parse_python(a);
        let tree_b = parse_python(b);
        let ha = hash_sound_module(a.as_bytes(), tree_a.root_node(), &profile);
        let hb = hash_sound_module(b.as_bytes(), tree_b.root_node(), &profile);
        assert_ne!(ha, hb);
    }

    #[test]
    fn rust_doc_comments_are_attributes_but_plain_comments_are_layout() {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::RUST.into())
            .expect("rust grammar");
        let profile = SoundStructuralProfile::for_language("rust");
        let mut hash = |src: &str| {
            let tree = parser.parse(src, None).expect("parse");
            hash_sound_module(src.as_bytes(), tree.root_node(), &profile)
        };
        assert_eq!(hash("fn f() {}\n"), hash("// plain\nfn f() {}\n"));
        assert_ne!(hash("/// doc a\nfn f() {}\n"), hash("/// doc b\nfn f() {}\n"));
        assert_ne!(hash("fn f() {}\n"), hash("//! crate doc\nfn f() {}\n"));
    }

    #[test]
    fn java_comments_are_layout() {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::JAVA.into())
            .expect("java grammar");
        let profile = SoundStructuralProfile::for_language("java");
        let mut hash = |src: &str| {
            let tree = parser.parse(src, None).expect("parse");
            hash_sound_module(src.as_bytes(), tree.root_node(), &profile)
        };
        assert_eq!(
            hash("class T { int f() { return 1; } }\n"),
            hash("class T {\n  /** doc */\n  // note\n  int f() { /* inline */ return 1; }\n}\n")
        );
    }

    #[test]
    fn go_embed_directive_is_retained() {
        let mut parser = Parser::new();
        parser.set_language(&crate::grammars::GO.into()).expect("go grammar");
        let a = "package p\n\n//go:embed assets\nvar x string\n";
        let b = "package p\n\n//go:embed other\nvar x string\n";
        let profile = SoundStructuralProfile::for_language("go");
        let tree_a = parser.parse(a, None).expect("parse");
        let tree_b = parser.parse(b, None).expect("parse");
        assert_ne!(
            hash_sound_module(a.as_bytes(), tree_a.root_node(), &profile),
            hash_sound_module(b.as_bytes(), tree_b.root_node(), &profile)
        );
    }
}
