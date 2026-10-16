//! Structured serialization framework for AST normalization.

/// Structured representation of serialized AST nodes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenStream {
    pub node_type: Option<String>,
    pub children: Vec<TokenStream>,
    pub identifier: Option<String>,
    pub literal_type: Option<String>,
    pub literal_value: Option<String>,
    pub text_content: Option<String>,
    pub param_position: Option<usize>,
}

impl TokenStream {
    /// Whether this stream carries a retained node or token.
    pub fn has_content(&self) -> bool {
        self.node_type.is_some()
            || self.identifier.is_some()
            || self.literal_type.is_some()
            || self.text_content.is_some()
            || !self.children.is_empty()
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn node(node_type: impl Into<String>, children: Vec<TokenStream>) -> Self {
        Self {
            node_type: Some(node_type.into()),
            children,
            ..Self::default()
        }
    }

    pub fn identifier(name: impl Into<String>) -> Self {
        Self {
            identifier: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn literal(literal_type: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            literal_type: Some(literal_type.into()),
            literal_value: Some(value.into()),
            ..Self::default()
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text_content: Some(text.into()),
            ..Self::default()
        }
    }

    pub fn with_param_position(mut self, position: usize) -> Self {
        self.param_position = Some(position);
        self
    }

    /// Convert to canonical string format (must match Python `TokenStream.to_canonical_string`).
    pub fn to_canonical_string(&self) -> String {
        if let Some(ref identifier) = self.identifier {
            if let Some(pos) = self.param_position {
                return format!("(param:{identifier}:pos_{pos})");
            }
            return format!("(identifier:{identifier})");
        }

        if let (Some(ref literal_type), Some(ref literal_value)) = (&self.literal_type, &self.literal_value) {
            return format!("({literal_type}:{literal_value})");
        }

        if let Some(ref text_content) = self.text_content {
            return format!("({text_content})");
        }

        if let Some(ref node_type) = self.node_type {
            if self.children.is_empty() {
                return format!("({node_type})");
            }

            let parts: Vec<String> = self
                .children
                .iter()
                .filter_map(|child| {
                    let s = child.to_canonical_string();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                })
                .collect();

            if parts.is_empty() {
                return format!("({node_type})");
            }
            return format!("({node_type} {})", parts.join(" "));
        }

        String::new()
    }

    pub fn is_empty(&self) -> bool {
        self.node_type.is_none()
            && self.identifier.is_none()
            && self.literal_type.is_none()
            && self.text_content.is_none()
            && self.children.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::TokenStream;

    #[test]
    fn canonical_identifier() {
        assert_eq!(
            TokenStream::identifier("arg_0").to_canonical_string(),
            "(identifier:arg_0)"
        );
    }

    #[test]
    fn canonical_parameter_reference() {
        assert_eq!(
            TokenStream::identifier("arg_0")
                .with_param_position(0)
                .to_canonical_string(),
            "(param:arg_0:pos_0)"
        );
    }

    #[test]
    fn canonical_literal() {
        assert_eq!(
            TokenStream::literal("number", "42").to_canonical_string(),
            "(number:42)"
        );
    }

    #[test]
    fn canonical_node_with_children() {
        let stream = TokenStream::node(
            "function_definition",
            vec![TokenStream::identifier("foo"), TokenStream::literal("number", "1")],
        );
        assert_eq!(
            stream.to_canonical_string(),
            "(function_definition (identifier:foo) (number:1))"
        );
    }

    #[test]
    fn canonical_leaf_node() {
        assert_eq!(TokenStream::node("pass", vec![]).to_canonical_string(), "(pass)");
    }

    #[test]
    fn empty_stream() {
        assert_eq!(TokenStream::new().to_canonical_string(), "");
        assert!(TokenStream::new().is_empty());
    }
}
