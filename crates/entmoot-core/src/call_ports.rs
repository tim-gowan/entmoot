//! Shared AST field adapter for syntax sites. No name binding or transfer inference.
use entmoot_contracts::{
    ArgumentSite, ArgumentSiteKind as Kind, CallSitePorts, SiteCompleteness, SiteLimitation, SiteSpan,
};
use tree_sitter::Node;

fn span(node: Node<'_>) -> SiteSpan {
    SiteSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}

pub(crate) fn extract(node: &Node<'_>, source: &[u8]) -> Option<CallSitePorts> {
    let mut cursor = node.walk();
    let arguments = node.child_by_field_name("arguments").or_else(|| {
        node.named_children(&mut cursor)
            .find(|n| matches!(n.kind(), "arguments" | "argument_list"))
    });
    let function = node.child_by_field_name("function");
    // Java's method invocation separates object and method name. C# invocation
    // grammars expose the expression as the first named child.
    let function = function
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.named_child(0));
    let function = function?;
    let selector = if function.kind() == "generic_function" {
        function.child_by_field_name("function").unwrap_or(function)
    } else {
        function
    };
    let receiver = node.child_by_field_name("object").or_else(|| {
        if matches!(
            selector.kind(),
            "member_expression"
                | "subscript_expression"
                | "attribute"
                | "field_expression"
                | "selector_expression"
                | "member_access_expression"
        ) {
            selector
                .child_by_field_name("object")
                .or_else(|| selector.child_by_field_name("value"))
                .or_else(|| selector.child_by_field_name("operand"))
                .or_else(|| selector.child_by_field_name("expression"))
                .or_else(|| selector.child_by_field_name("argument"))
        } else {
            None
        }
    });
    let callee = SiteSpan {
        start_byte: receiver.map_or(function.start_byte(), |n| n.start_byte()),
        end_byte: function.end_byte(),
    };
    let mut ports = CallSitePorts {
        callee,
        receiver: receiver.map(span),
        arguments: Vec::new(),
        result: span(*node),
        completeness: SiteCompleteness::Complete,
        limitations: Vec::new(),
    };
    if node.has_error() {
        ports.limitations.push(SiteLimitation::InvalidSyntax);
    }
    if let Some(arguments) = arguments.filter(|n| matches!(n.kind(), "arguments" | "argument_list")) {
        let mut cursor = arguments.walk();
        let mut needs_argument = true;
        for arg in arguments.children(&mut cursor) {
            if matches!(arg.kind(), "comment" | "line_comment" | "block_comment") {
                continue;
            }
            if arg.kind() == "," {
                // Some recovery grammars accept argument holes without an ERROR
                // node. Never call a collapsed ordinal list complete syntax.
                if needs_argument && !ports.limitations.contains(&SiteLimitation::InvalidSyntax) {
                    ports.limitations.push(SiteLimitation::InvalidSyntax);
                }
                needs_argument = true;
                continue;
            }
            if !arg.is_named() {
                continue;
            }
            needs_argument = false;
            if ports.arguments.len() == 256 {
                ports.limitations.push(SiteLimitation::ArgumentBudget);
                break;
            }
            let mut name_node = None;
            let kind = match arg.kind() {
                "keyword_argument" => {
                    name_node = arg.child_by_field_name("name");
                    Kind::Named
                }
                "list_splat" | "spread_element" | "variadic_argument" => Kind::Spread,
                "dictionary_splat" => Kind::KeywordSpread,
                "argument" => {
                    name_node = arg.child_by_field_name("name");
                    if name_node.is_some() {
                        Kind::Named
                    } else {
                        Kind::Positional
                    }
                }
                _ => Kind::Positional,
            };
            let name = name_node
                .and_then(|n| source.get(n.byte_range()))
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .and_then(|name| {
                    if name.len() > 1024 {
                        ports.limitations.push(SiteLimitation::OversizedName);
                        None
                    } else {
                        Some(name.to_owned())
                    }
                });
            ports.arguments.push(ArgumentSite {
                position: ports.arguments.len(),
                name,
                kind,
                span: span(arg),
            });
        }
    } else {
        ports.limitations.push(SiteLimitation::UnsupportedShape);
    }
    if !ports.limitations.is_empty() {
        ports.completeness = SiteCompleteness::Partial;
    }
    Some(ports)
}
