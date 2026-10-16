//! Go AST visitor.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::serialization::TokenStream;
use crate::string_escape::decode_unicode_escape_bytes;
use crate::visitor::{BaseVisitor, NodeVisitor, RecursionDepthError};
use entmoot_contracts::StructuralHashPolicy;

use super::normalizer::GoNormalizer;
use super::scope_analyzer::GoScopeAnalyzer;

const TYPE_SKIP_KINDS: &[&str] = &[
    "type_identifier",
    "qualified_type",
    "pointer_type",
    "array_type",
    "slice_type",
    "map_type",
    "channel_type",
    "function_type",
    "interface_type",
    "struct_type",
];

struct ParamInfo {
    identifier: Option<String>,
    type_name: Option<String>,
    is_pointer: bool,
}

enum MethodChildResult {
    One(TokenStream),
    Many(Vec<TokenStream>),
}

/// Visitor for Go AST nodes using structured serialization.
pub struct GoNodeVisitor<'a> {
    pub base: BaseVisitor,
    pub source: &'a [u8],
    pub normalizer: Option<&'a GoNormalizer>,
    package_aliases: HashMap<String, String>,
    package_vars: HashSet<String>,
    param_types: HashMap<String, String>,
    type_param_map: HashMap<String, String>,
    struct_fields: HashMap<String, Vec<String>>,
}

impl<'a> GoNodeVisitor<'a> {
    pub fn new(policy: Option<StructuralHashPolicy>, source: &'a [u8], normalizer: Option<&'a GoNormalizer>) -> Self {
        Self {
            base: BaseVisitor::new(None, policy),
            source,
            normalizer,
            package_aliases: HashMap::new(),
            package_vars: HashSet::new(),
            param_types: HashMap::new(),
            type_param_map: HashMap::new(),
            struct_fields: HashMap::new(),
        }
    }

    fn in_parameter_list(&self) -> bool {
        self.base.state.in_parameter_list
    }

    fn set_in_parameter_list(&mut self, value: bool) {
        self.base.state.in_parameter_list = value;
    }

    fn var_maps(&self) -> &Vec<HashMap<String, String>> {
        &self.base.state.var_maps
    }

    fn arg_maps(&self) -> &Vec<HashMap<String, String>> {
        &self.base.state.arg_maps
    }

    fn var_counter(&self) -> usize {
        self.base.state.var_counter
    }

    fn set_var_counter(&mut self, value: usize) {
        self.base.state.var_counter = value;
    }

    fn arg_counter(&self) -> usize {
        self.base.state.arg_counter
    }

    fn set_arg_counter(&mut self, value: usize) {
        self.base.state.arg_counter = value;
    }

    fn current_var_map_mut(&mut self) -> &mut HashMap<String, String> {
        self.base.state.current_var_map_mut()
    }

    fn current_arg_map(&self) -> &HashMap<String, String> {
        self.base.state.current_arg_map()
    }

    fn register_variable_if_new(&mut self, identifier_text: &str) -> String {
        self.base.register_variable_if_new(identifier_text)
    }

    // ------------------------------------------------------------------
    // Comments / punctuation
    // ------------------------------------------------------------------

    fn visit_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "comment")
    }

    fn visit_line_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "line_comment")
    }

    fn visit_block_comment(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.visit_comment_kind(node, "block_comment")
    }

    fn visit_error(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(child_stream) = self.visit(&child)? {
                if child_stream.node_type.as_deref() == Some("statement_list") {
                    children_streams.extend(child_stream.children);
                } else {
                    children_streams.push(child_stream);
                }
            }
        }
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("statement_list", children_streams)))
        }
    }

    fn visit_statement_list(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == ";" {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                if child_stream.text_content.as_deref() != Some(";") {
                    children_streams.push(child_stream);
                }
            }
        }
        Ok(Some(TokenStream::node("statement_list", children_streams)))
    }

    // ------------------------------------------------------------------
    // Imports / package
    // ------------------------------------------------------------------

    fn visit_package_clause(&mut self, _node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        Ok(None)
    }

    fn extract_string_literal(path_text: &str) -> String {
        if (path_text.starts_with('"') && path_text.ends_with('"'))
            || (path_text.starts_with('`') && path_text.ends_with('`'))
        {
            path_text[1..path_text.len() - 1].to_string()
        } else {
            path_text.to_string()
        }
    }

    fn canonical_import_name(path: &str) -> String {
        path.rsplit('/').next().unwrap_or(path).to_string()
    }

    fn track_go_import_usage(&mut self, name: &str) {
        let canonical = self
            .package_aliases
            .get(name)
            .map(|path| Self::canonical_import_name(path))
            .unwrap_or_else(|| name.to_string());
        if self.base.file_imports.contains(&canonical) {
            self.base.track_callable_import_usage(&canonical);
        }
    }

    fn process_import_spec(&mut self, spec: &Node) {
        let mut alias = None;
        let mut package_path = None;
        let mut cursor = spec.walk();
        for spec_child in spec.children(&mut cursor) {
            match spec_child.kind() {
                "package_identifier" => {
                    alias = self.source_text(&spec_child);
                }
                "interpreted_string_literal" | "raw_string_literal" => {
                    if let Some(path_text) = self.source_text(&spec_child) {
                        package_path = Some(Self::extract_string_literal(&path_text));
                    }
                }
                _ => {}
            }
        }
        if let Some(path) = package_path {
            let default_alias = Self::canonical_import_name(&path);
            self.base.track_file_import(default_alias.clone());
            match alias.as_deref() {
                Some(".") => {
                    // Dot import: Module, do not invent members.
                    self.base.track_import_marker(
                        path.clone(),
                        None,
                        entmoot_contracts::ImportKind::Module,
                        spec.start_byte(),
                        spec.end_byte(),
                    );
                }
                Some(a) => {
                    self.base.track_import_marker(
                        path.clone(),
                        Some(a.to_string()),
                        entmoot_contracts::ImportKind::Alias,
                        spec.start_byte(),
                        spec.end_byte(),
                    );
                }
                None => {
                    self.base.track_import_marker(
                        path.clone(),
                        None,
                        entmoot_contracts::ImportKind::Module,
                        spec.start_byte(),
                        spec.end_byte(),
                    );
                }
            }
            if let Some(a) = alias {
                self.package_aliases.insert(a, path);
            } else {
                self.package_aliases.insert(default_alias, path);
            }
        }
    }

    fn emit_import_token(&self, spec: &Node) -> Option<TokenStream> {
        let mut cursor = spec.walk();
        for spec_child in spec.children(&mut cursor) {
            if matches!(spec_child.kind(), "interpreted_string_literal" | "raw_string_literal") {
                if let Some(path_text) = self.source_text(&spec_child) {
                    let package_path = Self::extract_string_literal(&path_text);
                    return Some(TokenStream::literal("import", package_path));
                }
            }
        }
        None
    }

    fn visit_import_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut import_tokens = Vec::new();
        let mut cursor = node.walk();
        for import_child in node.children(&mut cursor) {
            if import_child.kind() == "import_spec_list" {
                let mut sc = import_child.walk();
                for spec in import_child.children(&mut sc) {
                    if spec.kind() == "import_spec" {
                        self.process_import_spec(&spec);
                        if let Some(token) = self.emit_import_token(&spec) {
                            import_tokens.push(token);
                        }
                    }
                }
            } else if import_child.kind() == "import_spec" {
                self.process_import_spec(&import_child);
                if let Some(token) = self.emit_import_token(&import_child) {
                    import_tokens.push(token);
                }
            }
        }
        Ok(Some(TokenStream::node("import_declaration", import_tokens)))
    }

    // ------------------------------------------------------------------
    // Functions / methods / parameters
    // ------------------------------------------------------------------

    fn extract_callable_name(&self, node: &Node) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "field_identifier") {
                if let Some(t) = self.source_text(&child) {
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
        }
        "<anonymous>".to_string()
    }

    /// Streams for callable hashing: exclude the function/method name so differently
    /// named callables with the same body share a hash.
    fn visit_function_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let name = self.extract_callable_name(node);
        self.base
            .begin_callable(name, "function", node.start_byte(), node.end_byte());

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.type_param_map.clear();
        self.base.preflight_callable_scope(&GoScopeAnalyzer, node, self.source);

        let children_streams = self.process_function_children(node)?;

        self.base.exit_callable_scope();
        self.base.exit_scope();

        self.base.end_callable();

        Ok(Some(TokenStream::node("function_declaration", children_streams)))
    }

    fn process_function_children(&mut self, node: &Node) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut seen_parameter_list = false;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "comment" | "line_comment" | "block_comment") {
                continue;
            }

            if child.kind() == "type_parameter_list" {
                if let Some(s) = self.visit(&child)? {
                    children_streams.push(s);
                }
                continue;
            }

            if child.kind() == "parameter_list" && !seen_parameter_list {
                self.set_in_parameter_list(true);
                children_streams.extend(self.process_parameter_list(&child, false)?);
                self.set_in_parameter_list(false);
                seen_parameter_list = true;
                continue;
            }

            if seen_parameter_list && matches!(child.kind(), "type_identifier" | "qualified_type" | "parameter_list") {
                if let Some(s) = self.extract_return_type(&child)? {
                    children_streams.push(s);
                }
                continue;
            }

            if matches!(child.kind(), "type_identifier" | "qualified_type") {
                if let Some(s) = self.visit(&child)? {
                    children_streams.push(s);
                }
                continue;
            }

            if let Some(s) = self.visit(&child)? {
                children_streams.push(s);
            }
        }

        Ok(children_streams)
    }

    fn process_parameter_list(
        &mut self,
        node: &Node,
        is_receiver: bool,
    ) -> Result<Vec<TokenStream>, RecursionDepthError> {
        let mut param_parts = Vec::new();
        let mut cursor = node.walk();
        for param in node.children(&mut cursor) {
            if param.kind() == "parameter_declaration" {
                if let Some(part) = self.process_single_parameter(&param, is_receiver)? {
                    param_parts.push(part);
                }
            }
        }
        Ok(param_parts)
    }

    fn process_single_parameter(
        &mut self,
        param: &Node,
        is_receiver: bool,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        let info = self.extract_parameter_info(param, is_receiver)?;
        let normalized_name = match &info.identifier {
            Some(id) if id != "_" && !id.is_empty() => {
                if self.arg_maps().is_empty() {
                    return Ok(None);
                }
                self.normalize_parameter_name(id)
            }
            _ => {
                if self.arg_maps().is_empty() {
                    self.base.enter_scope("function");
                }
                let temp = format!("arg_{}", self.arg_counter());
                self.set_arg_counter(self.arg_counter() + 1);
                temp
            }
        };

        if let Some(ref param_type) = info.type_name {
            self.param_types.insert(normalized_name.clone(), param_type.clone());
        }

        if is_receiver && info.is_pointer {
            let value = if let Some(ref t) = info.type_name {
                format!("{normalized_name}:{t}")
            } else {
                format!("{normalized_name}:pointer")
            };
            return Ok(Some(TokenStream::literal("parameter", value)));
        }

        if let Some(ref t) = info.type_name {
            Ok(Some(TokenStream::literal(
                "parameter",
                format!("{normalized_name}:{t}"),
            )))
        } else {
            Ok(Some(TokenStream::literal("parameter", normalized_name)))
        }
    }

    fn normalize_parameter_name(&mut self, identifier: &str) -> String {
        self.base.normalize_parameter_name(identifier)
    }

    fn extract_parameter_info(&mut self, param: &Node, is_receiver: bool) -> Result<ParamInfo, RecursionDepthError> {
        let mut info = ParamInfo {
            identifier: None,
            type_name: None,
            is_pointer: false,
        };

        if is_receiver {
            info.is_pointer = self.has_pointer_type(param);
            if info.is_pointer {
                info.type_name = self.extract_receiver_type(param);
            }
        }

        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            match param_child.kind() {
                "identifier" if !self.arg_maps().is_empty() => {
                    info.identifier = self.source_text(&param_child);
                }
                "channel_type" => {
                    let direction = self.extract_channel_direction(&param_child);
                    let element = self.extract_channel_element_type(&param_child);
                    info.type_name = Some(match (direction, element) {
                        (Some(d), Some(e)) => format!("channel_{d}_{e}"),
                        (None, Some(e)) => format!("channel_bidirectional_{e}"),
                        (Some(d), None) => format!("channel_{d}"),
                        (None, None) => "channel_bidirectional".to_string(),
                    });
                }
                "slice_type" => {
                    if let Some(stream) = self.visit(&param_child)? {
                        info.type_name = Some(
                            Self::extract_type_string_from_stream(&stream).unwrap_or_else(|| "slice_type".to_string()),
                        );
                    } else {
                        info.type_name = Some("slice_type".to_string());
                    }
                }
                "map_type" => {
                    if let Some(stream) = self.visit(&param_child)? {
                        info.type_name = Some(
                            Self::extract_type_string_from_stream(&stream).unwrap_or_else(|| "map_type".to_string()),
                        );
                    } else {
                        info.type_name = Some("map_type".to_string());
                    }
                }
                "qualified_type" => {
                    if let Some(t) = self.extract_qualified_type_name(&param_child) {
                        info.type_name = Some(t);
                    }
                }
                "type_identifier" if info.type_name.is_none() => {
                    if let Some(t) = self.source_text(&param_child) {
                        if !t.is_empty() {
                            info.type_name = Some(t);
                        }
                    }
                }
                "pointer_type" => {
                    if let Some(t) = self.extract_pointer_type(&param_child) {
                        info.type_name = Some(t);
                    }
                }
                _ => {}
            }
        }

        Ok(info)
    }

    fn extract_channel_direction(&self, channel_node: &Node) -> Option<String> {
        let mut chan_index = None;
        let mut arrow_index = None;
        let mut cursor = channel_node.walk();
        for (i, child) in channel_node.children(&mut cursor).enumerate() {
            let text = self.source_text(&child).unwrap_or_default();
            if text == "chan" || child.kind() == "chan" {
                chan_index = Some(i);
            } else if text == "<-" || child.kind() == "<-" {
                arrow_index = Some(i);
            }
        }
        match (chan_index, arrow_index) {
            (Some(c), Some(a)) if a < c => Some("receive_only".to_string()),
            (Some(c), Some(a)) if a > c => Some("send_only".to_string()),
            _ => None,
        }
    }

    fn extract_channel_element_type(&self, channel_node: &Node) -> Option<String> {
        let mut cursor = channel_node.walk();
        for child in channel_node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" => {
                    if let Some(t) = self.source_text(&child) {
                        if !t.is_empty() {
                            return Some(t);
                        }
                    }
                }
                "qualified_type" => {
                    if let Some(t) = self.extract_qualified_type_name(&child) {
                        return Some(t);
                    }
                }
                "slice_type" => return Some("slice_type".to_string()),
                "map_type" => return Some("map_type".to_string()),
                "pointer_type" => {
                    if let Some(t) = self.extract_pointer_type(&child) {
                        return Some(t);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn has_pointer_type(&self, param: &Node) -> bool {
        let mut cursor = param.walk();
        let children: Vec<Node> = param.children(&mut cursor).collect();
        children.iter().any(|c| c.kind() == "pointer_type")
    }

    fn extract_receiver_type(&self, param: &Node) -> Option<String> {
        let mut cursor = param.walk();
        for param_child in param.children(&mut cursor) {
            if param_child.kind() == "pointer_type" {
                let mut pc = param_child.walk();
                for ptr_child in param_child.children(&mut pc) {
                    if ptr_child.kind() == "type_identifier" {
                        if let Some(t) = self.source_text(&ptr_child) {
                            if !t.is_empty() {
                                return Some(format!("pointer_{t}"));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Type name for logic-address scope from the method receiver parameter list.
    fn extract_method_receiver_scope(&self, node: &Node) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "parameter_list" {
                continue;
            }
            let mut pc = child.walk();
            for param in child.children(&mut pc) {
                if param.kind() != "parameter_declaration" {
                    continue;
                }
                let mut pdc = param.walk();
                for part in param.children(&mut pdc) {
                    match part.kind() {
                        "type_identifier" => {
                            if let Some(t) = self.source_text(&part) {
                                if !t.is_empty() {
                                    return Some(t);
                                }
                            }
                        }
                        "pointer_type" => {
                            let mut ptrc = part.walk();
                            for ptr_child in part.children(&mut ptrc) {
                                if ptr_child.kind() == "type_identifier" {
                                    if let Some(t) = self.source_text(&ptr_child) {
                                        if !t.is_empty() {
                                            return Some(t);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Only the first parameter_list is the receiver.
            break;
        }
        None
    }

    fn extract_pointer_type(&self, ptr_child: &Node) -> Option<String> {
        let mut cursor = ptr_child.walk();
        for child in ptr_child.children(&mut cursor) {
            match child.kind() {
                "slice_type" | "map_type" | "channel_type" => return Some(child.kind().to_string()),
                "qualified_type" => {
                    if let Some(q) = self.extract_qualified_type_name(&child) {
                        return Some(format!("pointer_{q}"));
                    }
                }
                "type_identifier" => {
                    if let Some(t) = self.source_text(&child) {
                        if !t.is_empty() {
                            return Some(format!("pointer_{t}"));
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn extract_qualified_type_name(&self, node: &Node) -> Option<String> {
        if node.kind() != "qualified_type" {
            return None;
        }
        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "package_identifier" | "type_identifier") {
                if let Some(t) = self.source_text(&child) {
                    parts.push(t);
                }
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("."))
        }
    }

    fn extract_return_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match node.kind() {
            "type_identifier" => {
                if let Some(t) = self.source_text(node) {
                    if !t.is_empty() {
                        return Ok(Some(TokenStream::literal("return_type", t)));
                    }
                }
            }
            "qualified_type" => {
                if let Some(t) = self.extract_qualified_type_name(node) {
                    return Ok(Some(TokenStream::literal("return_type", t)));
                }
            }
            "parameter_list" => {
                let mut return_parts = Vec::new();
                let mut cursor = node.walk();
                for param in node.children(&mut cursor) {
                    if param.kind() != "parameter_declaration" {
                        continue;
                    }
                    let info = self.extract_parameter_info(&param, false)?;
                    if let Some(ref id) = info.identifier {
                        if id != "_" {
                            let normalized = if !self.var_maps().is_empty() {
                                let already = self
                                    .base
                                    .state
                                    .var_maps
                                    .last()
                                    .map(|m| m.contains_key(id))
                                    .unwrap_or(false);
                                if !already {
                                    let n = format!("var_{}", self.var_counter());
                                    self.current_var_map_mut().insert(id.clone(), n.clone());
                                    self.set_var_counter(self.var_counter() + 1);
                                    n
                                } else {
                                    self.base
                                        .state
                                        .var_maps
                                        .last()
                                        .and_then(|m| m.get(id))
                                        .cloned()
                                        .unwrap_or_else(|| id.clone())
                                }
                            } else if let Some(ref t) = info.type_name {
                                return_parts.push(format!("{id}:{t}"));
                                continue;
                            } else {
                                return_parts.push(id.clone());
                                continue;
                            };
                            if let Some(ref t) = info.type_name {
                                return_parts.push(format!("{normalized}:{t}"));
                            } else {
                                return_parts.push(normalized);
                            }
                            continue;
                        }
                    }
                    if let Some(ref t) = info.type_name {
                        return_parts.push(t.clone());
                    }
                }
                if !return_parts.is_empty() {
                    return Ok(Some(TokenStream::literal("return_type", return_parts.join(","))));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn visit_method_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let name = self.extract_callable_name(node);
        let receiver_scope = self.extract_method_receiver_scope(node);
        if let Some(ref recv) = receiver_scope {
            self.base.push_definition_scope(recv);
        }
        self.base
            .begin_callable(name, "method", node.start_byte(), node.end_byte());

        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.type_param_map.clear();
        self.base.preflight_callable_scope(&GoScopeAnalyzer, node, self.source);

        let mut children_streams = Vec::new();
        let mut seen_receiver = false;
        let mut seen_parameter_list = false;

        // Pre-scan to mark receiver / params like Python (flags updated before processing each child)
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in &children {
            let mut sr = seen_receiver;
            let mut sp = seen_parameter_list;
            if child.kind() == "parameter_list" && !sr {
                sr = true;
            } else if child.kind() == "parameter_list" && sr && !sp {
                sp = true;
            }

            // Process with flags as they were BEFORE this child's update in Python:
            // Python updates flags first, then calls _process_method_declaration_child
            // with the UPDATED flags. Wait - looking at Python again:
            //
            // if child.type == 'parameter_list' and not seen_receiver:
            //     seen_receiver = True
            // elif child.type == 'parameter_list' and seen_receiver and not seen_parameter_list:
            //     seen_parameter_list = True
            // child_result = self._process_method_declaration_child(child, seen_receiver, seen_parameter_list)
            //
            // So when processing the first parameter_list, seen_receiver is True when passed!
            // That means `_process_method_declaration_child` checks:
            //   if child.type == 'parameter_list' and not seen_receiver:  -> False (already True)
            //   if child.type == 'parameter_list' and seen_receiver and not seen_parameter_list: -> True
            // So the FIRST parameter_list is treated as regular parameters, NOT receiver!
            //
            // That seems like a bug in Python... Let me re-read.
            //
            // Actually when first parameter_list:
            //   seen_receiver was False, so first branch sets seen_receiver = True
            //   Then process with seen_receiver=True, seen_parameter_list=False
            //   In _process: `if child.type == 'parameter_list' and not seen_receiver` -> False
            //   `if child.type == 'parameter_list' and seen_receiver and not seen_parameter_list` -> True
            //   So it processes as regular parameters with is_receiver=False!
            //
            // That can't be right for methods... Unless the receiver handling never works?
            // Wait, look again at the condition for receiver in _process:
            //   if child.type == 'parameter_list' and not seen_receiver:
            //
            // But seen_receiver is already True when called. So receiver path is NEVER taken!
            //
            // Unless... there's a bug. Let me look at an earlier version or tests for methods.
            // Looking at the loop again - maybe I should process BEFORE updating flags to match
            // intended semantics (first list = receiver). The Python code looks buggy.
            // I'll implement the INTENDED semantics: first parameter_list is receiver.

            let _ = (sr, sp);
        }

        seen_receiver = false;
        seen_parameter_list = false;
        for child in &children {
            if let Some(result) = self.process_method_declaration_child(child, seen_receiver, seen_parameter_list)? {
                match result {
                    MethodChildResult::One(s) => children_streams.push(s),
                    MethodChildResult::Many(parts) => children_streams.extend(parts),
                }
            }
            if child.kind() == "parameter_list" && !seen_receiver {
                seen_receiver = true;
            } else if child.kind() == "parameter_list" && seen_receiver && !seen_parameter_list {
                seen_parameter_list = true;
            }
        }

        self.base.exit_callable_scope();
        self.base.exit_scope();

        self.base.end_callable();
        if receiver_scope.is_some() {
            self.base.pop_definition_scope();
        }

        Ok(Some(TokenStream::node("method_declaration", children_streams)))
    }

    fn process_method_declaration_child(
        &mut self,
        child: &Node,
        seen_receiver: bool,
        seen_parameter_list: bool,
    ) -> Result<Option<MethodChildResult>, RecursionDepthError> {
        if matches!(child.kind(), "comment" | "line_comment" | "block_comment") {
            return Ok(None);
        }

        if child.kind() == "type_parameter_list" {
            return Ok(self.visit(child)?.map(MethodChildResult::One));
        }

        // First parameter_list = receiver (process BEFORE flag update)
        if child.kind() == "parameter_list" && !seen_receiver {
            self.set_in_parameter_list(true);
            let parts = self.process_parameter_list(child, true)?;
            self.set_in_parameter_list(false);
            return Ok(Some(MethodChildResult::Many(parts)));
        }

        if child.kind() == "parameter_list" && seen_receiver && !seen_parameter_list {
            self.set_in_parameter_list(true);
            let parts = self.process_parameter_list(child, false)?;
            self.set_in_parameter_list(false);
            return Ok(Some(MethodChildResult::Many(parts)));
        }

        if seen_parameter_list && matches!(child.kind(), "type_identifier" | "qualified_type" | "parameter_list") {
            if let Some(s) = self.extract_return_type(child)? {
                return Ok(Some(MethodChildResult::One(s)));
            }
        }

        if child.kind() == "type_identifier" {
            let type_text = self.source_text(child).unwrap_or_default();
            if let Some(normalized) = self.type_param_map.get(&type_text) {
                return Ok(Some(MethodChildResult::One(TokenStream::identifier(
                    normalized.clone(),
                ))));
            }
            return Ok(None);
        }

        if child.kind() == "qualified_type" {
            return Ok(None);
        }

        Ok(self.visit(child)?.map(MethodChildResult::One))
    }

    fn visit_func_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // The body executes in its own callable scope. Recording its calls as
        // the enclosing function's calls would invent ownership, including for
        // literals that are only returned or stored and never invoked here.
        let enclosing = self
            .base
            .current_callable
            .as_ref()
            .map(|callable| callable.name.clone());
        if let Some(name) = &enclosing {
            self.base.push_definition_scope(name.clone());
        }
        self.base
            .begin_callable("<anonymous>", "function", node.start_byte(), node.end_byte());
        self.base.enter_scope("function");
        self.set_in_parameter_list(false);
        self.base.preflight_callable_scope(&GoScopeAnalyzer, node, self.source);
        let children_streams = self.process_function_children(node)?;
        self.base.exit_callable_scope();
        self.base.exit_scope();
        self.base.end_callable();
        if enclosing.is_some() {
            self.base.pop_definition_scope();
        }
        Ok(Some(TokenStream::node("func_literal", children_streams)))
    }

    // ------------------------------------------------------------------
    // Identifiers
    // ------------------------------------------------------------------

    fn visit_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let identifier_text = self.source_text(node).unwrap_or_default();

        if self.base.file_imports.contains(&identifier_text) {
            self.track_go_import_usage(&identifier_text);
        }

        if let Some(special) = self.handle_special_identifier(&identifier_text) {
            return Ok(Some(special));
        }

        if self.var_maps().is_empty() || self.in_parameter_list() {
            return self.visit_generic(node);
        }

        Ok(Some(self.normalize_identifier_in_scope(&identifier_text)))
    }

    fn handle_special_identifier(&self, identifier_text: &str) -> Option<TokenStream> {
        if identifier_text == "_" {
            return Some(TokenStream::identifier("_"));
        }
        if let Some(normalized) = self.type_param_map.get(identifier_text) {
            return Some(TokenStream::identifier(normalized.clone()));
        }
        None
    }

    fn normalize_identifier_in_scope(&mut self, identifier_text: &str) -> TokenStream {
        if self.package_vars.contains(identifier_text) {
            return TokenStream::identifier(identifier_text.to_string());
        }

        if !self.is_bound_variable(identifier_text) {
            return TokenStream::identifier(identifier_text.to_string());
        }

        let name = self.register_variable_if_new(identifier_text);
        TokenStream::identifier(name)
    }

    fn is_bound_variable(&self, identifier_text: &str) -> bool {
        self.base.is_allowed_local(identifier_text)
    }

    fn visit_type_identifier(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let type_text = self.source_text(node).unwrap_or_default();
        if let Some(normalized) = self.type_param_map.get(&type_text) {
            return Ok(Some(TokenStream::identifier(normalized.clone())));
        }
        Ok(Some(TokenStream::literal("type_identifier", type_text)))
    }

    // ------------------------------------------------------------------
    // Variable declarations
    // ------------------------------------------------------------------

    fn visit_short_var_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if !self.var_maps().is_empty() {
            let mut cursor = node.walk();
            for assign_child in node.children(&mut cursor) {
                if assign_child.kind() == "identifier" {
                    let identifier_text = self.source_text(&assign_child).unwrap_or_default();
                    if identifier_text == "_" {
                        continue;
                    }
                    let in_arg = self.current_arg_map().contains_key(&identifier_text);
                    let in_var = self
                        .base
                        .state
                        .var_maps
                        .last()
                        .map(|m| m.contains_key(&identifier_text))
                        .unwrap_or(false);
                    if !in_arg && !in_var {
                        let n = format!("var_{}", self.var_counter());
                        self.current_var_map_mut().insert(identifier_text, n);
                        self.set_var_counter(self.var_counter() + 1);
                    }
                }
            }
        }
        let children = self.visit_children(node)?;
        Ok(Some(TokenStream::node("short_var_declaration", children)))
    }

    fn visit_var_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let is_package_level = self.var_maps().is_empty();
        if is_package_level {
            self.handle_package_level_var(node);
        } else {
            self.handle_function_scoped_var(node);
        }
        let children = self.visit_children(node)?;
        Ok(Some(TokenStream::node("var_declaration", children)))
    }

    fn handle_package_level_var(&mut self, node: &Node) {
        let mut cursor = node.walk();
        for assign_child in node.children(&mut cursor) {
            if assign_child.kind() == "var_spec" {
                let mut sc = assign_child.walk();
                for spec_child in assign_child.children(&mut sc) {
                    if spec_child.kind() == "identifier" {
                        if let Some(id) = self.source_text(&spec_child) {
                            if id != "_" {
                                self.package_vars.insert(id);
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_function_scoped_var(&mut self, node: &Node) {
        let mut cursor = node.walk();
        for assign_child in node.children(&mut cursor) {
            if assign_child.kind() == "var_spec" {
                let mut sc = assign_child.walk();
                for spec_child in assign_child.children(&mut sc) {
                    if spec_child.kind() == "identifier" {
                        let identifier_text = self.source_text(&spec_child).unwrap_or_default();
                        if identifier_text == "_" {
                            continue;
                        }
                        let in_arg = self.current_arg_map().contains_key(&identifier_text);
                        let in_var = self
                            .base
                            .state
                            .var_maps
                            .last()
                            .map(|m| m.contains_key(&identifier_text))
                            .unwrap_or(false);
                        if !in_arg && !in_var {
                            let n = format!("var_{}", self.var_counter());
                            self.current_var_map_mut().insert(identifier_text, n);
                            self.set_var_counter(self.var_counter() + 1);
                        }
                    }
                }
            }
        }
    }

    fn visit_var_spec(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                let identifier_text = self.source_text(&child).unwrap_or_default();
                if identifier_text != "_" && !self.var_maps().is_empty() {
                    let already = self
                        .base
                        .state
                        .var_maps
                        .last()
                        .map(|m| m.contains_key(&identifier_text))
                        .unwrap_or(false);
                    if !already {
                        let n = format!("var_{}", self.var_counter());
                        self.current_var_map_mut().insert(identifier_text.clone(), n);
                        self.set_var_counter(self.var_counter() + 1);
                    }
                    let normalized = self
                        .base
                        .state
                        .var_maps
                        .last()
                        .and_then(|m| m.get(&identifier_text))
                        .cloned()
                        .unwrap_or(identifier_text);
                    children_streams.push(TokenStream::identifier(normalized));
                } else {
                    children_streams.push(TokenStream::identifier(identifier_text));
                }
            } else if TYPE_SKIP_KINDS.contains(&child.kind()) {
                continue;
            } else if let Some(res) = self.visit(&child)? {
                children_streams.push(res);
            }
        }
        Ok(Some(TokenStream::node("var_spec", children_streams)))
    }

    // ------------------------------------------------------------------
    // Blocks
    // ------------------------------------------------------------------

    fn visit_block(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.base.enter_scope("block");
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "{" | "}" | ";") {
                continue;
            }
            if let Some(child_stream) = self.visit(&child)? {
                if child_stream.node_type.as_deref() == Some("statement_list") {
                    children_streams.extend(
                        child_stream
                            .children
                            .into_iter()
                            .filter(|c| c.text_content.as_deref() != Some(";")),
                    );
                } else {
                    children_streams.push(child_stream);
                }
            }
        }
        self.base.exit_scope();

        if children_streams.len() == 1 && children_streams[0].node_type.as_deref() == Some("block") {
            children_streams = children_streams.remove(0).children;
        }

        Ok(Some(TokenStream::node("block", children_streams)))
    }

    // ------------------------------------------------------------------
    // Literals
    // ------------------------------------------------------------------

    fn normalize_string_literal(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        if text.starts_with('`') && text.ends_with('`') {
            return text[1..text.len() - 1].to_string();
        }
        if text.starts_with('"') && text.ends_with('"') {
            let content = &text[1..text.len() - 1];
            return decode_go_string_escape(content);
        }
        text.to_string()
    }

    fn visit_string_like(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            let normalized = self.normalize_string_literal(&text);
            return Ok(Some(TokenStream::literal("string", normalized)));
        }
        Ok(None)
    }

    fn visit_int_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(mut text) = self.source_text(node) {
            if let Some(normalizer) = self.normalizer {
                text = normalizer.normalize_integer_literal_text(&text);
            }
            return Ok(Some(TokenStream::literal("integer", text)));
        }
        Ok(None)
    }

    fn visit_float_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("float", text)));
        }
        Ok(None)
    }

    fn visit_rune_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        if let Some(text) = self.source_text(node) {
            return Ok(Some(TokenStream::literal("rune", text)));
        }
        Ok(None)
    }

    // ------------------------------------------------------------------
    // Composite literals / structs / interfaces
    // ------------------------------------------------------------------

    fn visit_composite_literal(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut values_only = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                if let Some(s) = self.visit(&child)? {
                    children_streams.push(s);
                }
            } else if child.kind() == "literal_value" {
                self.process_literal_value(&child, &mut values_only, node)?;
            } else if let Some(s) = self.visit(&child)? {
                children_streams.push(s);
            }
        }
        children_streams.extend(values_only);
        Ok(Some(TokenStream::node("composite_literal", children_streams)))
    }

    fn process_literal_value(
        &mut self,
        literal_value_node: &Node,
        values_only: &mut Vec<TokenStream>,
        composite_literal_node: &Node,
    ) -> Result<(), RecursionDepthError> {
        let struct_type_name = self.get_composite_literal_type_name(composite_literal_node);
        let struct_field_names = struct_type_name
            .as_ref()
            .and_then(|n| self.struct_fields.get(n))
            .cloned()
            .unwrap_or_default();

        let mut positional_index = 0usize;
        let mut keyed_elements = Vec::new();
        let mut keyed_field_names = Vec::new();

        let mut cursor = literal_value_node.walk();
        for literal_child in literal_value_node.children(&mut cursor) {
            match literal_child.kind() {
                "keyed_element" => {
                    if let Some(keyed_stream) = self.visit_keyed_element(&literal_child)? {
                        for keyed_child in &keyed_stream.children {
                            if keyed_child.literal_type.as_deref() == Some("field_name") {
                                if let Some(ref v) = keyed_child.literal_value {
                                    keyed_field_names.push(v.clone());
                                }
                                break;
                            }
                        }
                        keyed_elements.push(keyed_stream);
                    }
                    positional_index += 1;
                }
                "literal_element" => {
                    positional_index = self.process_positional_element_in_literal(
                        &literal_child,
                        values_only,
                        &struct_field_names,
                        positional_index,
                    )?;
                }
                "{" | "}" | "," | ":" => {}
                _ => {
                    if let Some(value_stream) = self.visit(&literal_child)? {
                        values_only.push(value_stream);
                    }
                }
            }
        }

        if !keyed_elements.is_empty() && !struct_field_names.is_empty() {
            let keyed_field_set: HashSet<&str> = keyed_field_names.iter().map(|s| s.as_str()).collect();
            let struct_field_set: HashSet<&str> = struct_field_names.iter().map(|s| s.as_str()).collect();
            if keyed_field_set == struct_field_set && keyed_field_names.len() == struct_field_names.len() {
                let mut field_to_element = HashMap::new();
                for (i, keyed_stream) in keyed_elements.into_iter().enumerate() {
                    field_to_element.insert(keyed_field_names[i].clone(), keyed_stream);
                }
                for struct_field in &struct_field_names {
                    if let Some(el) = field_to_element.remove(struct_field) {
                        values_only.push(el);
                    }
                }
            } else {
                values_only.extend(keyed_elements);
            }
        } else {
            values_only.extend(keyed_elements);
        }
        Ok(())
    }

    fn process_positional_element_in_literal(
        &mut self,
        literal_element: &Node,
        values_only: &mut Vec<TokenStream>,
        struct_field_names: &[String],
        positional_index: usize,
    ) -> Result<usize, RecursionDepthError> {
        if let Some(value_stream) = self.visit(literal_element)? {
            let field_name_text = if positional_index < struct_field_names.len() {
                struct_field_names[positional_index].clone()
            } else {
                format!("field_{positional_index}")
            };
            let field_name = TokenStream::literal("field_name", field_name_text);
            values_only.push(TokenStream::node("keyed_element", vec![field_name, value_stream]));
        }
        Ok(positional_index + 1)
    }

    fn visit_keyed_element(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut field_name = None;
        let mut value_stream = None;
        let mut found_colon = false;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                ":" => {
                    found_colon = true;
                }
                "," => {}
                _ if !found_colon => {
                    if let Some(name) = self.extract_field_name_from_node(&child) {
                        field_name = Some(TokenStream::literal("field_name", name));
                    }
                }
                _ => {
                    if let Some(visited) = self.visit(&child)? {
                        value_stream = Some(visited);
                    }
                }
            }
        }

        if let Some(fn_) = field_name {
            children_streams.push(fn_);
        }
        if let Some(vs) = value_stream {
            children_streams.push(vs);
        }
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("keyed_element", children_streams)))
        }
    }

    fn extract_field_name_from_node(&self, node: &Node) -> Option<String> {
        if matches!(node.kind(), "identifier" | "field_identifier") {
            return self.source_text(node);
        }
        if let Some(text) = self.source_text(node) {
            if !text.is_empty() && !matches!(text.as_str(), ":" | "," | "=" | "{" | "}") {
                return Some(text);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "field_identifier") {
                return self.source_text(&child);
            }
            if let Some(text) = self.source_text(&child) {
                if !text.is_empty() && !matches!(text.as_str(), ":" | "," | "=" | "{" | "}") {
                    return Some(text);
                }
            }
        }
        None
    }

    fn get_composite_literal_type_name(&self, node: &Node) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                let t = self.source_text(&child)?;
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
        None
    }

    fn extract_struct_field_names(&self, struct_type_node: &Node) -> Vec<String> {
        let mut field_names = Vec::new();
        let mut cursor = struct_type_node.walk();
        for child in struct_type_node.children(&mut cursor) {
            if child.kind() == "field_declaration_list" {
                let mut fc = child.walk();
                for field_child in child.children(&mut fc) {
                    if field_child.kind() == "field_declaration" {
                        if let Some(name) = self.extract_field_name_from_declaration(&field_child) {
                            field_names.push(name);
                        }
                    }
                }
            }
        }
        field_names
    }

    fn extract_field_name_from_declaration(&self, field_declaration: &Node) -> Option<String> {
        let mut cursor = field_declaration.walk();
        for field_decl_child in field_declaration.children(&mut cursor) {
            match field_decl_child.kind() {
                "field_identifier" => return self.source_text(&field_decl_child),
                "identifier" => {
                    let name = self.source_text(&field_decl_child)?;
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
                "field_name" => {
                    let mut fnc = field_decl_child.walk();
                    for fn_child in field_decl_child.children(&mut fnc) {
                        if matches!(fn_child.kind(), "identifier" | "field_identifier") {
                            if let Some(name) = self.source_text(&fn_child) {
                                if !name.is_empty() {
                                    return Some(name);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn visit_type_declaration(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut type_name = None;
        let mut struct_type_node = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_spec" {
                let mut sc = child.walk();
                for spec_child in child.children(&mut sc) {
                    if spec_child.kind() == "type_identifier" {
                        type_name = self.source_text(&spec_child);
                    } else if spec_child.kind() == "struct_type" {
                        struct_type_node = Some(spec_child);
                    }
                }
            }
        }
        if let (Some(name), Some(struct_node)) = (type_name, struct_type_node) {
            let field_names = self.extract_struct_field_names(&struct_node);
            if !field_names.is_empty() {
                self.struct_fields.insert(name, field_names);
            }
        }
        let children = self.visit_children(node)?;
        Ok(Some(TokenStream::node("type_declaration", children)))
    }

    fn visit_interface_type(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "method_spec" | "method_elem" => {
                    if let Some(s) = self.process_method_spec(&child)? {
                        children_streams.push(s);
                    }
                }
                "{" | "}" | "interface" => {}
                _ => {
                    if let Some(s) = self.visit(&child)? {
                        children_streams.push(s);
                    }
                }
            }
        }
        Ok(Some(TokenStream::node("interface_type", children_streams)))
    }

    fn process_method_spec(&mut self, method_spec_node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut method_name = None;
        let mut parameter_list = None;
        let mut return_types = Vec::new();
        let mut parameter_lists = Vec::new();
        let mut found_method_name = false;

        let mut cursor = method_spec_node.walk();
        for child in method_spec_node.children(&mut cursor) {
            match child.kind() {
                "field_identifier" | "identifier" => {
                    method_name = self.source_text(&child);
                    found_method_name = true;
                }
                "function_type" => {
                    let mut fc = child.walk();
                    for func_child in child.children(&mut fc) {
                        if func_child.kind() == "parameter_list" {
                            parameter_lists.push(func_child);
                        }
                    }
                }
                "parameter_list" => parameter_lists.push(child),
                "type_identifier" if found_method_name => {
                    if let Some(return_type_stream) = self.visit(&child)? {
                        if let Some(type_str) = Self::extract_type_string_from_stream(&return_type_stream) {
                            return_types.push(TokenStream::literal("return_type", type_str));
                        }
                    }
                }
                "parameter_declaration" => {
                    if let Some(param_stream) = self.process_single_parameter(&child, false)? {
                        let list = parameter_list.get_or_insert_with(|| TokenStream::node("parameter_list", vec![]));
                        list.children.push(param_stream);
                    }
                }
                "slice_type" | "array_type" => {
                    if let Some(param_type_stream) = self.visit(&child)? {
                        if let Some(type_str) = Self::extract_type_string_from_stream(&param_type_stream) {
                            if parameter_list.is_none() {
                                parameter_list = Some(TokenStream::node(
                                    "parameter_list",
                                    vec![TokenStream::literal("parameter", format!("arg_0:{type_str}"))],
                                ));
                            }
                        }
                    }
                }
                "{" | "}" | "," | ";" | "(" | ")" => {}
                "qualified_type" | "pointer_type" | "map_type" | "channel_type" => {
                    if let Some(return_type_stream) = self.visit(&child)? {
                        if let Some(type_str) = Self::extract_type_string_from_stream(&return_type_stream) {
                            return_types.push(TokenStream::literal("return_type", type_str));
                        }
                    }
                }
                _ => {}
            }
        }

        if !parameter_lists.is_empty() {
            let param_streams = self.process_parameter_list(&parameter_lists[0], false)?;
            if !param_streams.is_empty() {
                parameter_list = Some(TokenStream::node("parameter_list", param_streams));
            }
            if parameter_lists.len() > 1 {
                let return_param_streams = self.process_parameter_list(&parameter_lists[1], false)?;
                for return_param in return_param_streams {
                    if let Some(ref param_value) = return_param.literal_value {
                        if let Some((_, param_type)) = param_value.split_once(':') {
                            return_types.push(TokenStream::literal("return_type", param_type.to_string()));
                        } else {
                            return_types.push(TokenStream::literal("return_type", param_value.clone()));
                        }
                    }
                }
            }
        }

        let Some(method_name) = method_name else {
            return Ok(None);
        };

        let mut method_children = vec![TokenStream::literal("method_name", method_name)];
        method_children.push(parameter_list.unwrap_or_else(|| TokenStream::node("parameter_list", vec![])));
        if return_types.is_empty() {
            method_children.push(TokenStream::node("return_type_list", vec![]));
        } else {
            method_children.push(TokenStream::node("return_type_list", return_types));
        }
        Ok(Some(TokenStream::node("method_spec", method_children)))
    }

    fn extract_type_string_from_stream(stream: &TokenStream) -> Option<String> {
        if let (Some(ref lt), Some(ref lv)) = (&stream.literal_type, &stream.literal_value) {
            let _ = lt;
            return Some(lv.clone());
        }
        if let Some(ref id) = stream.identifier {
            return Some(id.clone());
        }
        if let Some(ref node_type) = stream.node_type {
            if !stream.children.is_empty() {
                let child_strs: Vec<String> = stream
                    .children
                    .iter()
                    .filter_map(Self::extract_type_string_from_stream)
                    .collect();
                if !child_strs.is_empty() {
                    return Some(format!("{node_type}({})", child_strs.join(",")));
                }
            }
            return Some(node_type.clone());
        }
        None
    }

    // ------------------------------------------------------------------
    // Range / type params / assertions / calls
    // ------------------------------------------------------------------

    fn visit_range_clause(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut container_type = None;
        let mut found_range = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "range" {
                found_range = true;
                continue;
            }
            if found_range {
                if let Some(detected) = self.detect_container_type(&child) {
                    container_type = Some(detected);
                }
                if let Some(s) = self.visit(&child)? {
                    children_streams.push(s);
                }
            } else if let Some(s) = self.visit(&child)? {
                children_streams.push(s);
            }
        }
        if let Some(ct) = container_type {
            children_streams.push(TokenStream::literal("container_type", ct));
        }
        Ok(Some(TokenStream::node("range_clause", children_streams)))
    }

    fn detect_container_type(&self, child: &Node) -> Option<String> {
        if child.kind() == "identifier" {
            if let Some(t) = self.detect_container_type_from_identifier(child) {
                return Some(t);
            }
        }
        if matches!(child.kind(), "slice_type" | "map_type" | "channel_type") {
            return Some(child.kind().to_string());
        }
        self.detect_container_type_from_children(child)
    }

    fn detect_container_type_from_identifier(&self, child: &Node) -> Option<String> {
        let identifier_text = self.source_text(child)?;
        if self.var_maps().is_empty() {
            return None;
        }
        if let Some(normalized) = self.current_arg_map().get(&identifier_text) {
            if let Some(t) = self.param_types.get(normalized) {
                return Some(t.clone());
            }
        }
        for arg_map in self.arg_maps() {
            if let Some(normalized) = arg_map.get(&identifier_text) {
                if let Some(t) = self.param_types.get(normalized) {
                    return Some(t.clone());
                }
            }
        }
        None
    }

    fn detect_container_type_from_children(&self, child: &Node) -> Option<String> {
        let mut cursor = child.walk();
        for type_child in child.children(&mut cursor) {
            if matches!(type_child.kind(), "slice_type" | "map_type" | "channel_type") {
                return Some(type_child.kind().to_string());
            }
            if let Some(type_text) = self.source_text(&type_child) {
                let lower = type_text.to_lowercase();
                if lower.contains("[]") || lower.contains("slice") {
                    return Some("slice_type".to_string());
                }
                if lower.contains("map[") || lower.contains("map") {
                    return Some("map_type".to_string());
                }
                if lower.contains("chan") || lower.contains("channel") {
                    return Some("channel_type".to_string());
                }
            }
        }
        None
    }

    fn visit_type_parameter_list(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut type_param_counter = 0usize;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "[" | "]" | ",") {
                continue;
            }
            if child.kind() == "type_parameter_declaration" {
                let mut type_param_name = None;
                let mut constraint_info = None;
                let mut found_type_identifier = false;
                let mut pc = child.walk();
                for param_child in child.children(&mut pc) {
                    if matches!(param_child.kind(), "type_identifier" | "identifier") && !found_type_identifier {
                        if let Some(name) = self.source_text(&param_child) {
                            if !name.is_empty() {
                                let normalized = format!("type_param_{type_param_counter}");
                                self.type_param_map.insert(name.clone(), normalized);
                                type_param_counter += 1;
                                type_param_name = Some(name);
                                found_type_identifier = true;
                            }
                        }
                    } else if param_child.kind() == "type_constraint" {
                        constraint_info = self.extract_type_constraint(&param_child)?;
                    } else if param_child.kind() == "type_identifier" && found_type_identifier {
                        if let Some(constraint_text) = self.source_text(&param_child) {
                            if matches!(constraint_text.as_str(), "any" | "comparable") {
                                constraint_info = Some(constraint_text);
                            }
                        }
                    } else if matches!(
                        param_child.kind(),
                        "union_type" | "approximation_constraint" | "qualified_type"
                    ) {
                        constraint_info = self.extract_type_constraint(&param_child)?;
                    }
                }
                if let Some(ref name) = type_param_name {
                    let normalized = self
                        .type_param_map
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| format!("type_param_{type_param_counter}"));
                    let param_value = if let Some(ref c) = constraint_info {
                        format!("{normalized}:{c}")
                    } else {
                        normalized
                    };
                    children_streams.push(TokenStream::literal("type_parameter", param_value));
                }
            } else if let Some(child_stream) = self.visit(&child)? {
                if child_stream.node_type.as_deref() != Some("[") && child_stream.node_type.as_deref() != Some("]") {
                    children_streams.push(child_stream);
                }
            }
        }
        Ok(Some(TokenStream::node("type_parameter_list", children_streams)))
    }

    fn extract_type_constraint(&mut self, constraint_node: &Node) -> Result<Option<String>, RecursionDepthError> {
        if constraint_node.kind() == "type_identifier" {
            return Ok(self.source_text(constraint_node).filter(|t| !t.is_empty()));
        }

        let mut constraint_parts = Vec::new();
        let mut cursor = constraint_node.walk();
        for child in constraint_node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" => {
                    if let Some(t) = self.source_text(&child) {
                        if !t.is_empty() {
                            constraint_parts.push(t);
                        }
                    }
                }
                "union_type" => {
                    let mut union_parts = Vec::new();
                    let mut uc = child.walk();
                    for union_child in child.children(&mut uc) {
                        if union_child.kind() == "type_identifier" {
                            if let Some(t) = self.source_text(&union_child) {
                                if !t.is_empty() {
                                    union_parts.push(t);
                                }
                            }
                        } else if union_child.kind() == "|" {
                            union_parts.push("|".to_string());
                        }
                    }
                    if !union_parts.is_empty() {
                        constraint_parts.push(union_parts.join("|"));
                    }
                }
                "approximation_constraint" => {
                    let mut approx_parts = Vec::new();
                    let mut ac = child.walk();
                    for approx_child in child.children(&mut ac) {
                        if approx_child.kind() == "~" {
                            approx_parts.push("~".to_string());
                        } else if approx_child.kind() == "type_identifier" {
                            if let Some(t) = self.source_text(&approx_child) {
                                if !t.is_empty() {
                                    approx_parts.push(t);
                                }
                            }
                        }
                    }
                    if !approx_parts.is_empty() {
                        constraint_parts.push(approx_parts.join(""));
                    }
                }
                "qualified_type" => {
                    if let Some(q) = self.extract_qualified_type_name(&child) {
                        constraint_parts.push(q);
                    }
                }
                "|" => constraint_parts.push("|".to_string()),
                "{" | "}" | "," | ";" | "(" | ")" => {}
                _ => {
                    if let Some(constraint_stream) = self.visit(&child)? {
                        if let Some(type_str) = Self::extract_type_string_from_stream(&constraint_stream) {
                            constraint_parts.push(type_str);
                        }
                    }
                }
            }
        }

        if constraint_parts.is_empty() {
            Ok(None)
        } else if constraint_parts.len() == 1 {
            Ok(Some(constraint_parts.remove(0)))
        } else {
            Ok(Some(constraint_parts.join("|")))
        }
    }

    fn visit_type_assertion_like(
        &mut self,
        node: &Node,
        node_type: &str,
    ) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut type_info = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(
                child.kind(),
                "type_identifier" | "qualified_type" | "slice_type" | "map_type" | "channel_type" | "pointer_type"
            ) {
                if let Some(type_stream) = self.visit(&child)? {
                    if let Some(ref id) = type_stream.identifier {
                        type_info = Some(id.clone());
                    } else if let Some(text) = self.source_text(&child) {
                        type_info = Some(text);
                    }
                } else if let Some(text) = self.source_text(&child) {
                    type_info = Some(text);
                }
            } else if let Some(child_stream) = self.visit(&child)? {
                children_streams.push(child_stream);
            }
        }
        if let Some(ti) = type_info {
            children_streams.push(TokenStream::literal("type", ti));
        }
        Ok(Some(TokenStream::node(node_type, children_streams)))
    }

    fn extract_qualified_name_go(&self, node: &Node) -> Option<String> {
        let normalizer = self.normalizer?;
        Some(normalizer.extract_qualified_name_go(
            node,
            self.source,
            self.var_maps(),
            self.arg_maps(),
            &self.package_aliases,
        ))
    }

    fn visit_call_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        // Every callable-valued expression is a syntax site, even when it has
        // no identifier that the bounded lexical resolver can select.
        let function = node.child_by_field_name("function");
        if let Some(callee) = function.as_ref().and_then(|value| self.source_text(value)) {
            self.base.track_call_node(callee, node, self.source);
        }
        let mut call_parts = Vec::new();
        let mut function_name = None;
        let mut qualified_name = None;
        let mut cursor = node.walk();
        for call_child in node.children(&mut cursor) {
            match call_child.kind() {
                "identifier" => {
                    function_name = self.extract_function_name_from_call(&call_child);
                    if let Some(ref name) = function_name {
                        self.track_go_import_usage(name);
                    }
                }
                "selector_expression" => {
                    if call_child
                        .child_by_field_name("operand")
                        .is_some_and(|operand| operand.kind() != "identifier")
                    {
                        if let Some(receiver) = self.visit(&call_child)? {
                            call_parts.push(TokenStream::node("callee_expression", vec![receiver]));
                        }
                    }
                    let mut sc = call_child.walk();
                    for sel_child in call_child.children(&mut sc) {
                        if sel_child.kind() == "identifier" {
                            if let Some(id) = self.source_text(&sel_child) {
                                self.track_go_import_usage(&id);
                            }
                            break;
                        }
                    }
                    qualified_name = self.extract_qualified_name_go(&call_child);
                }
                "argument_list" => {
                    if let Some(arg_serialized) = self.visit(&call_child)? {
                        call_parts.push(arg_serialized);
                    }
                }
                _ if function.is_some_and(|value| value.id() == call_child.id()) => {
                    if let Some(value) = self.visit(&call_child)? {
                        call_parts.push(TokenStream::node("callee_expression", vec![value]));
                    }
                }
                _ => {}
            }
        }
        let callee = qualified_name.or(function_name);
        if let Some(name) = callee {
            call_parts.push(TokenStream::literal("call", name));
        }
        if call_parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("call_expression", call_parts)))
        }
    }

    fn extract_function_name_from_call(&self, call_child: &Node) -> Option<String> {
        if call_child.kind() != "identifier" {
            return None;
        }
        let mut function_name = self.source_text(call_child)?;
        if !self.var_maps().is_empty() {
            if let Some(n) = self.current_arg_map().get(&function_name) {
                function_name = n.clone();
            } else if let Some(map) = self.var_maps().last() {
                if let Some(n) = map.get(&function_name) {
                    function_name = n.clone();
                }
            }
        }
        Some(function_name)
    }

    fn visit_binary_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(s) = self.visit(&child)? {
                children_streams.push(s);
            }
        }
        if children_streams.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TokenStream::node("binary_expression", children_streams)))
        }
    }

    fn visit_selector_expression(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let children = self.visit_children(node)?;
        Ok(Some(TokenStream::node("selector_expression", children)))
    }

    fn visit_go_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "go" {
                continue;
            }
            if let Some(s) = self.visit(&child)? {
                children_streams.push(s);
            }
        }
        Ok(Some(TokenStream::node("go_statement", children_streams)))
    }

    fn visit_defer_statement(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        let mut children_streams = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "defer" {
                continue;
            }
            if let Some(s) = self.visit(&child)? {
                children_streams.push(s);
            }
        }
        Ok(Some(TokenStream::node("defer_statement", children_streams)))
    }

    fn dispatch_visit(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        match node.kind() {
            "comment" => self.visit_comment(node),
            "line_comment" => self.visit_line_comment(node),
            "block_comment" => self.visit_block_comment(node),
            ";" | "semicolon" => Ok(None),
            "ERROR" => self.visit_error(node),
            "statement_list" => self.visit_statement_list(node),
            "package_clause" => self.visit_package_clause(node),
            "import_declaration" => self.visit_import_declaration(node),
            "function_declaration" => self.visit_function_declaration(node),
            "method_declaration" => self.visit_method_declaration(node),
            "func_literal" => self.visit_func_literal(node),
            "parameter_list" | "parameter_declaration" | "type_parameter_declaration" => Ok(None),
            "identifier" => self.visit_identifier(node),
            "type_identifier" => self.visit_type_identifier(node),
            "short_var_declaration" => self.visit_short_var_declaration(node),
            "var_declaration" => self.visit_var_declaration(node),
            "var_spec" => self.visit_var_spec(node),
            "block" => self.visit_block(node),
            "interpreted_string_literal" | "raw_string_literal" | "string_literal" => self.visit_string_like(node),
            "int_literal" => self.visit_int_literal(node),
            "float_literal" => self.visit_float_literal(node),
            "rune_literal" => self.visit_rune_literal(node),
            "composite_literal" => self.visit_composite_literal(node),
            "keyed_element" => self.visit_keyed_element(node),
            "type_declaration" => self.visit_type_declaration(node),
            "interface_type" => self.visit_interface_type(node),
            "method_elem" | "method_spec" => self.process_method_spec(node),
            "range_clause" => self.visit_range_clause(node),
            "type_parameter_list" => self.visit_type_parameter_list(node),
            "type_assertion" => self.visit_type_assertion_like(node, "type_assertion"),
            "type_assertion_expression" => self.visit_type_assertion_like(node, "type_assertion_expression"),
            "call_expression" => self.visit_call_expression(node),
            "binary_expression" => self.visit_binary_expression(node),
            "selector_expression" => self.visit_selector_expression(node),
            "go_statement" => self.visit_go_statement(node),
            "defer_statement" => self.visit_defer_statement(node),
            _ => self.visit_generic(node),
        }
    }
}

impl<'a> NodeVisitor for GoNodeVisitor<'a> {
    fn source(&self) -> &[u8] {
        self.source
    }
    fn context(&mut self) -> (&mut BaseVisitor, &[u8]) {
        (&mut self.base, self.source)
    }

    fn dispatch(&mut self, node: &Node) -> Result<Option<TokenStream>, RecursionDepthError> {
        self.dispatch_visit(node)
    }

    fn retain_empty_node(&self, kind: &str) -> bool {
        kind == "source_file"
    }
}

fn decode_go_string_escape(content: &str) -> String {
    decode_unicode_escape_bytes(content.as_bytes()).unwrap_or_else(|| content.to_string())
}
