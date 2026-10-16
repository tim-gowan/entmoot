//! Collects [`FileMarkers`](entmoot_contracts::FileMarkers) during the unified AST walk.

use entmoot_contracts::{
    BindingKind, BindingMarker, ControlSite, ControlSiteKind, DefMarker, ImportKind, ImportMarker, SiteSpan,
};

use crate::callable_context::CallableContext;

use crate::analysis::context::{FileAnalysisContext, PendingDeclaration};

/// Record a file-level import marker with kind, optional alias, and byte span.
pub fn record_import(
    ctx: &mut FileAnalysisContext,
    path: impl Into<String>,
    alias: Option<String>,
    kind: ImportKind,
    start_byte: usize,
    end_byte: usize,
) {
    if !ctx.collect_markers {
        return;
    }
    ctx.markers.imports.push(ImportMarker {
        path: path.into(),
        alias,
        kind,
        start_byte,
        end_byte,
    });
}

/// Record a callable definition marker.
pub fn record_callable_begin(ctx: &mut FileAnalysisContext, callable: &CallableContext) {
    if !ctx.collect_markers {
        return;
    }
    ctx.markers.definitions.push(DefMarker {
        name: callable.name.clone(),
        kind: callable.kind.as_str().to_string(),
        scope: callable.scope.clone(),
        start_byte: callable.start_byte,
        end_byte: callable.end_byte,
    });
}

/// Record a declaration whose binding identity is distinct from a callable body.
pub fn record_declaration(
    ctx: &mut FileAnalysisContext,
    name: impl Into<String>,
    kind: impl Into<String>,
    scope: Vec<String>,
    start_byte: usize,
    end_byte: usize,
    callable_start_byte: Option<usize>,
) {
    if !ctx.collect_markers {
        return;
    }
    ctx.declarations.push(PendingDeclaration {
        syntax: None,
        name: name.into(),
        kind: kind.into(),
        scope,
        span: SiteSpan { start_byte, end_byte },
        callable_start_byte,
    });
}

/// Record an alpha-rename binding.
pub fn record_binding(ctx: &mut FileAnalysisContext, binding: BindingMarker) {
    if ctx.collect_markers {
        ctx.markers.bindings.push(binding);
    }
}

/// Record a call expression site. Oversized callee spellings use an explicit
/// digest label, never a truncated name that could bind to another definition.
/// The exact source span remains available; this changes no structural tokens.
pub fn record_call_site(
    ctx: &mut FileAnalysisContext,
    owner: Option<&CallableContext>,
    callee: impl Into<String>,
    start_byte: usize,
    end_byte: usize,
) {
    if !ctx.collect_markers {
        return;
    }
    let callee = callee.into();
    let callee = if callee.len() > 1024 {
        format!(
            "<oversized-callee:{}>",
            entmoot_contracts::blake3_hex(callee.as_bytes())
        )
    } else {
        callee
    };
    ctx.markers.call_sites.push(entmoot_contracts::CallSiteMarker {
        callee,
        start_byte,
        end_byte,
        owner: owner.map(|callable| entmoot_contracts::DefinitionSiteRef {
            name: callable.name.clone(),
            scope: callable.scope.clone(),
            start_byte: callable.start_byte,
        }),
        ports: None,
    });
}

/// Record a control-structure site when entering a known construct kind.
pub fn record_control_site(ctx: &mut FileAnalysisContext, kind: &str, start_byte: usize, end_byte: usize) {
    if !ctx.collect_markers {
        return;
    }
    if let Some(site_kind) = control_site_kind_for_node(kind) {
        ctx.markers.control_sites.push(ControlSite {
            from_byte: start_byte,
            to_byte: end_byte,
            kind: site_kind,
        });
    }
}

fn control_site_kind_for_node(kind: &str) -> Option<ControlSiteKind> {
    match kind {
        "if_statement" | "if_expression" => Some(ControlSiteKind::Conditional),
        "while_statement"
        | "while_expression"
        | "do_statement"
        | "for_statement"
        | "for_in_statement"
        | "for_of_statement"
        | "for_expression"
        | "loop_expression"
        | "enhanced_for_statement"
        | "foreach_statement" => Some(ControlSiteKind::Loop),
        "switch_statement" | "switch_expression" | "expression_switch_statement" | "type_switch_statement" => {
            Some(ControlSiteKind::Switch)
        }
        "match_statement" | "match_expression" | "match_arm" => Some(ControlSiteKind::Match),
        "return_statement" | "return_expression" => Some(ControlSiteKind::Return),
        "break_statement" | "break_expression" => Some(ControlSiteKind::Break),
        "continue_statement" | "continue_expression" => Some(ControlSiteKind::Continue),
        "throw_statement" | "throw_expression" | "raise_statement" => Some(ControlSiteKind::Throw),
        _ => None,
    }
}

/// Build a binding marker from alpha-rename registration.
pub fn binding_marker_parameter(name: &str, normalized: &str, start_byte: Option<usize>) -> BindingMarker {
    BindingMarker {
        name: name.to_string(),
        normalized: Some(normalized.to_string()),
        binding_kind: BindingKind::Parameter,
        start_byte,
    }
}

/// Build a binding marker from alpha-rename registration.
pub fn binding_marker_local(name: &str, normalized: &str, start_byte: Option<usize>) -> BindingMarker {
    BindingMarker {
        name: name.to_string(),
        normalized: Some(normalized.to_string()),
        binding_kind: BindingKind::Local,
        start_byte,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::context::FileAnalysisContext;

    #[test]
    fn control_site_kinds_mapped_for_common_nodes() {
        let cases = [
            ("if_statement", ControlSiteKind::Conditional),
            ("for_of_statement", ControlSiteKind::Loop),
            ("enhanced_for_statement", ControlSiteKind::Loop),
            ("foreach_statement", ControlSiteKind::Loop),
            ("loop_expression", ControlSiteKind::Loop),
            ("type_switch_statement", ControlSiteKind::Switch),
            ("switch_expression", ControlSiteKind::Switch),
            ("match_statement", ControlSiteKind::Match),
            ("return_expression", ControlSiteKind::Return),
            ("break_expression", ControlSiteKind::Break),
            ("continue_expression", ControlSiteKind::Continue),
            ("raise_statement", ControlSiteKind::Throw),
        ];
        for (node_kind, expected) in cases {
            assert_eq!(control_site_kind_for_node(node_kind), Some(expected));
        }
        assert!(control_site_kind_for_node("identifier").is_none());
    }

    #[test]
    fn record_call_site_when_collecting() {
        let mut ctx = FileAnalysisContext::new(true);
        record_call_site(&mut ctx, None, "foo", 10, 20);
        assert_eq!(ctx.markers.call_sites.len(), 1);
        assert_eq!(ctx.markers.call_sites[0].callee, "foo");
        assert!(ctx.markers.call_sites[0].owner.is_none());
    }

    #[test]
    fn oversized_callee_spelling_retains_span_without_embedding_a_bundle() {
        let mut ctx = FileAnalysisContext::new(true);
        let expression = format!("(() => {{ /* {} */ }})", "unicode-λ".repeat(500));
        record_call_site(&mut ctx, None, expression.clone(), 10, expression.len() + 12);
        let marker = &ctx.markers.call_sites[0];
        assert_eq!(
            marker.callee,
            format!(
                "<oversized-callee:{}>",
                entmoot_contracts::blake3_hex(expression.as_bytes())
            )
        );
        assert_eq!((marker.start_byte, marker.end_byte), (10, expression.len() + 12));
    }
}
