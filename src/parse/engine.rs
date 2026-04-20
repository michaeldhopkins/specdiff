use crate::parse::registry::FrameworkDef;
use crate::parse::shared::SharedExampleRegistry;
use crate::parse::{SpecKind, SpecNode, SpecTree};
use tree_sitter::{Node, Parser};

pub fn parse_file(source: &str, path: &str, framework: &FrameworkDef) -> Option<SpecTree> {
    parse_file_with_shared(source, path, framework, None)
}

pub fn parse_file_with_shared(
    source: &str,
    path: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<SpecTree> {
    let mut parser = Parser::new();
    let language = language_for_framework(framework)?;
    parser.set_language(&language).ok()?;

    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let children = parse_children_with_shared(root, source, framework, shared);

    if children.is_empty() {
        return None;
    }

    Some(SpecTree {
        file_path: path.to_string(),
        framework: framework.name.clone(),
        root: children,
    })
}

pub fn language_for_framework(framework: &FrameworkDef) -> Option<tree_sitter::Language> {
    match framework.language.as_str() {
        "ruby" => Some(tree_sitter_ruby::LANGUAGE.into()),
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "elixir" => Some(tree_sitter_elixir::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "php" => Some(tree_sitter_php::LANGUAGE_PHP.into()),
        _ => None,
    }
}

pub fn parse_children(node: Node, source: &str, framework: &FrameworkDef) -> Vec<SpecNode> {
    parse_children_with_shared(node, source, framework, None)
}

pub fn parse_children_with_shared(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Vec<SpecNode> {
    let mut results = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if is_inert_container(child, framework) {
            continue;
        }
        if let Some(nodes) = try_match_inclusion(child, source, framework, shared) {
            results.extend(nodes);
        } else if let Some(spec_node) = try_match_node(child, source, framework, shared) {
            results.push(spec_node);
        } else if let Some(expanded) = try_expand_loop(child, source, framework, shared) {
            results.extend(expanded);
        } else {
            results.extend(parse_children_with_shared(child, source, framework, shared));
        }
    }

    results
}

fn is_inert_container(node: Node, framework: &FrameworkDef) -> bool {
    framework
        .inheritance
        .as_ref()
        .is_some_and(|inh| inh.inert_containers.iter().any(|k| k == node.kind()))
}

fn try_match_node(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<SpecNode> {
    if let Some(result) = try_match_dsl_node(node, source, framework, shared) {
        return Some(result);
    }
    if let Some(result) = try_match_marker_node(node, source, framework, shared) {
        return Some(result);
    }
    None
}

// Expand `.each do |var| ...end` (or `{ |var| ... }`) into one spec per
// literal collection element. Substitutes #{var} in the parsed body's spec
// names with each value.
//
// Supported receivers: %i[], %w[], plain arrays of symbols/strings/numbers,
// hash literals (whose pairs feed |k, v|), and arrays of nested arrays
// (whose inner arrays feed multi-arg block parameters).
//
// Limitations:
//   * Substitution is a literal `name.replace`; complex interpolation like
//     `#{var.method}` survives unsubstituted (we'd need to evaluate the
//     method call). Outer-scope variables referenced inside the body
//     cannot be resolved beyond what scope-walking constant collection
//     turns up.
//   * Only spec NAMES whose source string had real interpolation get
//     substituted (tracked via SpecNode::name_is_dynamic). A literal
//     `#{var}` typed inside a single-quoted string is left alone.
fn try_expand_loop(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<Vec<SpecNode>> {
    let loop_def = framework.loop_expansion.as_ref()?;
    if !loop_def.enabled || loop_def.each_method.is_empty() {
        return None;
    }

    if !is_dsl_call_kind(node.kind()) {
        return None;
    }
    let method = extract_method_name(node, source)?;
    if method != loop_def.each_method {
        return None;
    }

    let receiver = node.child_by_field_name("receiver")?;
    if !loop_def
        .literal_receiver_kinds
        .iter()
        .any(|k| k == receiver.kind())
    {
        return None;
    }

    let tuples = extract_literal_tuples(receiver, source);
    let block = node.child_by_field_name("block")?;
    let params = extract_block_parameters(block, source);
    let body = block.child_by_field_name("body")?;

    // Empty literal (`%i[]` or `{}`) means zero iterations at runtime —
    // return Some(empty) so callers don't fall through to recursion and
    // emit a phantom describe with the unsubstituted placeholder.
    if tuples.is_empty() {
        return Some(vec![]);
    }

    let base_specs = parse_children_with_shared(body, source, framework, shared);
    if base_specs.is_empty() {
        return None;
    }

    let scope_assignments = collect_in_scope_assignments(node, source);
    let loop_placeholders: Vec<String> = params
        .iter()
        .map(|p| format!("#{{{p}}}"))
        .collect();

    let mut expanded = Vec::with_capacity(tuples.len() * base_specs.len());
    for tuple in &tuples {
        let mut subs: Vec<(String, String)> = loop_placeholders
            .iter()
            .zip(tuple.iter())
            .map(|(p, v)| (p.clone(), v.clone()))
            .collect();
        subs.extend(scope_assignments.iter().cloned());
        for spec in &base_specs {
            expanded.push(substitute_spec_name(spec, &subs));
        }
    }
    Some(expanded)
}

// Walk previous siblings up the parent chain looking for `<ident> = "<literal>"`
// assignments. Bindings closer to the loop shadow further-out bindings. Used by
// loop expansion so a `PREFIX = "p"` defined alongside the each-loop substitutes
// into `#{PREFIX}` in spec names.
fn collect_in_scope_assignments(node: Node, source: &str) -> Vec<(String, String)> {
    let mut assignments = Vec::new();
    let mut cursor = Some(node);
    while let Some(n) = cursor {
        let mut prev = n.prev_sibling();
        while let Some(sib) = prev {
            if sib.kind() == "assignment"
                && let Some((name, value)) = extract_string_assignment(sib, source)
            {
                let placeholder = format!("#{{{name}}}");
                if !assignments.iter().any(|(p, _): &(String, String)| p == &placeholder) {
                    assignments.push((placeholder, value));
                }
            }
            prev = sib.prev_sibling();
        }
        cursor = n.parent();
    }
    assignments
}

fn extract_string_assignment(node: Node, source: &str) -> Option<(String, String)> {
    let left = node.child_by_field_name("left")?;
    let right = node.child_by_field_name("right")?;
    if left.kind() != "identifier" && left.kind() != "constant" {
        return None;
    }
    let name = node_text(left, source)?;
    let value = literal_atom_text(right, source)?;
    Some((name, value))
}

fn extract_literal_tuples(receiver: Node, source: &str) -> Vec<Vec<String>> {
    let mut tuples = Vec::new();
    let mut cursor = receiver.walk();
    for child in receiver.children(&mut cursor) {
        match child.kind() {
            "bare_symbol" | "bare_string" => {
                if let Some(text) = node_text(child, source) {
                    tuples.push(vec![text]);
                }
            }
            "simple_symbol" => {
                if let Some(text) = node_text(child, source) {
                    tuples.push(vec![text.trim_start_matches(':').to_string()]);
                }
            }
            "string" => {
                if let Some(text) = extract_string_content(child, source) {
                    tuples.push(vec![text]);
                }
            }
            "integer" | "float" => {
                if let Some(text) = node_text(child, source) {
                    tuples.push(vec![text]);
                }
            }
            "pair" => {
                // Hash element: |k, v|.
                if let Some(pair) = extract_pair_values(child, source) {
                    tuples.push(pair);
                }
            }
            "array" => {
                // Nested array element for paired/tuple iteration.
                let inner = extract_literal_tuples(child, source);
                let flat: Vec<String> = inner.into_iter().flatten().collect();
                if !flat.is_empty() {
                    tuples.push(flat);
                }
            }
            _ => {}
        }
    }
    tuples
}

fn extract_pair_values(pair: Node, source: &str) -> Option<Vec<String>> {
    let key = pair.child_by_field_name("key")?;
    let value = pair.child_by_field_name("value")?;
    let key_text = literal_atom_text(key, source)?;
    let value_text = literal_atom_text(value, source)?;
    Some(vec![key_text, value_text])
}

fn literal_atom_text(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "bare_symbol" | "bare_string" => node_text(node, source),
        "simple_symbol" => node_text(node, source).map(|t| t.trim_start_matches(':').to_string()),
        "hash_key_symbol" => node_text(node, source),
        "string" => extract_string_content(node, source),
        "integer" | "float" => node_text(node, source),
        _ => None,
    }
}

fn extract_block_parameters(block: Node, source: &str) -> Vec<String> {
    let Some(params) = block.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut cursor = params.walk();
    for child in params.children(&mut cursor) {
        if child.kind() == "identifier"
            && let Some(text) = node_text(child, source)
        {
            names.push(text);
        }
    }
    names
}

fn substitute_spec_name(spec: &SpecNode, subs: &[(String, String)]) -> SpecNode {
    let new_name = if spec.name_is_dynamic {
        let mut name = spec.name.clone();
        for (placeholder, value) in subs {
            name = name.replace(placeholder, value);
        }
        name
    } else {
        spec.name.clone()
    };
    SpecNode {
        name: new_name,
        kind: spec.kind.clone(),
        children: spec
            .children
            .iter()
            .map(|c| substitute_spec_name(c, subs))
            .collect(),
        line: spec.line,
        parameterized: spec.parameterized.clone(),
        name_is_dynamic: spec.name_is_dynamic,
    }
}

fn try_match_inclusion(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<Vec<SpecNode>> {
    let shared_def = framework.shared.as_ref()?;

    if !is_dsl_call_kind(node.kind()) {
        return None;
    }

    let method_name = extract_method_name(node, source)?;

    for inclusion in &shared_def.inclusion {
        if inclusion.ast_type != node.kind() {
            continue;
        }
        if !inclusion.method_names.contains(&method_name) {
            continue;
        }

        let name = extract_name(
            node,
            source,
            &inclusion.name_source,
            inclusion.name_source_type.as_deref(),
        )?;

        let name_is_dynamic = name_source_is_dynamic(node, &inclusion.name_source);

        let Some(registry) = shared else {
            let display = match inclusion.nesting.as_deref() {
                Some("nested") => inclusion
                    .nested_name_template
                    .as_deref()
                    .unwrap_or("{name}")
                    .replace("{name}", &name),
                _ => name,
            };
            return Some(vec![SpecNode {
                name: format!("\u{2026} {display}"),
                kind: SpecKind::SharedInclusion,
                children: vec![],
                line: node.start_position().row + 1,
                parameterized: None,
                name_is_dynamic,
            }]);
        };

        let specs = registry.get(&name)?;

        match inclusion.nesting.as_deref() {
            Some("nested") => {
                let group_name = inclusion
                    .nested_name_template
                    .as_deref()
                    .unwrap_or("{name}")
                    .replace("{name}", &name);
                return Some(vec![SpecNode {
                    name: group_name,
                    kind: SpecKind::Group,
                    children: specs.to_vec(),
                    line: node.start_position().row + 1,
                    parameterized: None,
                    name_is_dynamic,
                }]);
            }
            _ => {
                return Some(specs.to_vec());
            }
        }
    }

    None
}

fn try_match_dsl_node(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<SpecNode> {
    if !is_dsl_call_kind(node.kind()) {
        return None;
    }

    let method_name = extract_method_name(node, source)?;

    for group_def in &framework.group {
        if group_def.ast_type == node.kind()
            && group_def.method_names.contains(&method_name)
            && let Some(name) = extract_name(node, source, &group_def.name_source, group_def.name_source_type.as_deref())
        {
            let block_node = find_block(node);
            let children = if let Some(block) = block_node {
                parse_children_with_shared(block, source, framework, shared)
            } else {
                vec![]
            };
            return Some(SpecNode {
                name,
                kind: SpecKind::Group,
                children,
                line: node.start_position().row + 1,
                parameterized: None,
                name_is_dynamic: name_source_is_dynamic(node, &group_def.name_source),
            });
        }
    }

    for spec_def in &framework.spec {
        if spec_def.ast_type == node.kind() && spec_def.method_names.contains(&method_name) {
            let name = extract_name(node, source, &spec_def.name_source, spec_def.name_source_type.as_deref())
                .or_else(|| {
                    if spec_def.allow_anonymous {
                        Some(format!("anonymous spec at line {}", node.start_position().row + 1))
                    } else {
                        None
                    }
                });
            if let Some(name) = name {
                return Some(SpecNode {
                    name,
                    kind: SpecKind::Spec,
                    children: vec![],
                    line: node.start_position().row + 1,
                    parameterized: detect_parameterization(node, source, framework),
                    name_is_dynamic: name_source_is_dynamic(node, &spec_def.name_source),
                });
            }
        }
    }

    None
}

pub(crate) fn is_dsl_call_kind(kind: &str) -> bool {
    matches!(kind, "call" | "call_expression" | "function_call_expression")
}

fn name_source_is_dynamic(node: Node, name_source: &str) -> bool {
    if name_source != "first_argument" {
        return false;
    }
    let Some(args) = find_arguments(node) else {
        return false;
    };
    let Some(arg) = args.named_child(0) else {
        return false;
    };
    string_node_has_interpolation(unwrap_php_argument(arg))
}

pub fn extract_method_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "call" => {
            if let Some(method_node) = node.child_by_field_name("method") {
                return node_text(method_node, source);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return node_text(child, source);
                }
            }
            None
        }
        "call_expression" => {
            let function_node = node.child_by_field_name("function")?;
            if function_node.kind() == "call_expression" {
                return extract_method_name(function_node, source);
            }
            if function_node.kind() == "member_expression" {
                let base = base_object(function_node)?;
                return node_text(base, source);
            }
            node_text(function_node, source)
        }
        "function_call_expression" => {
            let function_node = node.child_by_field_name("function")?;
            node_text(function_node, source)
        }
        _ => None,
    }
}

fn base_object(node: Node) -> Option<Node> {
    let mut current = node;
    loop {
        if current.kind() != "member_expression" {
            return Some(current);
        }
        current = current.child_by_field_name("object")?;
    }
}

pub fn extract_name(
    node: Node,
    source: &str,
    name_source: &str,
    name_source_type: Option<&str>,
) -> Option<String> {
    match name_source {
        "first_argument" => {
            let args = find_arguments(node)?;
            let first_arg = unwrap_php_argument(args.named_child(0)?);
            match name_source_type {
                Some("string_literal") => extract_string_content(first_arg, source),
                Some("constant") => node_text(first_arg, source),
                _ => node_text(first_arg, source),
            }
        }
        "identifier" => {
            let name_node = node.child_by_field_name("name")?;
            node_text(name_node, source)
        }
        _ => None,
    }
}

fn unwrap_php_argument(node: Node) -> Node {
    if node.kind() == "argument" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                return child;
            }
        }
    }
    node
}

fn find_arguments(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "argument_list" | "arguments" => return Some(child),
            _ => {}
        }
    }
    None
}

pub fn find_block(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "block" | "do_block" | "statement_block" => return Some(child),
            "arguments" | "argument_list" => {
                let mut c2 = child.walk();
                for arg in child.children(&mut c2) {
                    match arg.kind() {
                        "arrow_function" | "function" => {
                            let mut c3 = arg.walk();
                            for inner in arg.children(&mut c3) {
                                if inner.kind() == "statement_block" {
                                    return Some(inner);
                                }
                            }
                        }
                        "func_literal" => {
                            return arg.child_by_field_name("body");
                        }
                        "argument" => {
                            let mut c3 = arg.walk();
                            for inner in arg.children(&mut c3) {
                                if inner.kind() == "anonymous_function" {
                                    return inner.child_by_field_name("body");
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn string_node_has_interpolation(node: Node) -> bool {
    if !matches!(
        node.kind(),
        "string" | "string_literal" | "interpreted_string_literal" | "encapsed_string"
    ) {
        return false;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|c| {
        matches!(c.kind(), "interpolation" | "template_substitution" | "variable_name")
    })
}

fn extract_string_content(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" | "string_literal" | "interpreted_string_literal" | "encapsed_string" => {
            let mut content_children: Vec<Node> = Vec::new();
            let mut has_dynamic = false;
            {
                let mut cursor = node.walk();
                for c in node.children(&mut cursor) {
                    match c.kind() {
                        "string_content"
                        | "string_fragment"
                        | "interpreted_string_literal_content"
                        | "quoted_content" => content_children.push(c),
                        "interpolation" | "template_substitution" | "variable_name" => {
                            has_dynamic = true;
                        }
                        _ => {}
                    }
                }
            }

            if !has_dynamic && content_children.len() == 1 {
                return node_text(content_children[0], source);
            }

            let text = node_text(node, source)?;
            Some(text.trim_matches(|c| c == '"' || c == '\'').to_string())
        }
        _ => None,
    }
}

pub fn node_text(node: Node, source: &str) -> Option<String> {
    source.get(node.byte_range()).map(|s| s.to_string())
}

fn try_match_marker_node(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<SpecNode> {
    for marker in &framework.marker {
        match marker.marker_type.as_str() {
            "attribute" => {
                if let Some(result) = try_match_attribute_marker(node, source, marker, framework, shared) {
                    return Some(result);
                }
            }
            "name_pattern" => {
                if let Some(result) = try_match_name_pattern_marker(node, source, marker, framework, shared) {
                    return Some(result);
                }
            }
            "annotation" => {
                if let Some(result) = try_match_annotation_marker(node, source, marker, framework, shared) {
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

fn try_match_annotation_marker(
    node: Node,
    source: &str,
    marker: &crate::parse::registry::MarkerDef,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<SpecNode> {
    let ast_kinds = framework.ast_kinds.as_ref()?;
    let target_kind = match marker.applies_to.as_str() {
        "function" | "method" => ast_kinds.method.as_deref().or(ast_kinds.function.as_deref())?,
        "class" => ast_kinds.class.as_deref()?,
        _ => return None,
    };

    if node.kind() != target_kind {
        return None;
    }

    let annotation_name = marker.marker_name.as_deref()?;
    if !has_annotation(node, source, annotation_name) {
        return None;
    }

    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source)?;
    let normalized = normalize_name(&name, framework);

    match marker.creates.as_str() {
        "spec" => Some(SpecNode {
            name: normalized,
            kind: SpecKind::Spec,
            children: vec![],
            line: node.start_position().row + 1,
            parameterized: detect_parameterization(node, source, framework),
            name_is_dynamic: false,
        }),
        "group" => {
            let mut children = inherited_specs(node, source, framework, shared);
            if let Some(body) = find_type_body(node, framework) {
                children.extend(parse_children_with_shared(body, source, framework, shared));
            }
            Some(SpecNode {
                name: normalized,
                kind: SpecKind::Group,
                children,
                line: node.start_position().row + 1,
                parameterized: None,
                name_is_dynamic: false,
            })
        }
        _ => None,
    }
}

fn has_annotation(node: Node, source: &str, name: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut mc = child.walk();
            for modifier in child.children(&mut mc) {
                if modifier.kind() == "marker_annotation" || modifier.kind() == "annotation" {
                    let text = node_text(modifier, source).unwrap_or_default();
                    let annotation_name = text.strip_prefix('@').unwrap_or(&text);
                    if annotation_name == name || annotation_name.ends_with(&format!(".{name}")) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn try_match_attribute_marker(
    node: Node,
    source: &str,
    marker: &crate::parse::registry::MarkerDef,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<SpecNode> {
    let target_kind = match marker.applies_to.as_str() {
        "function" => "function_item",
        "module" => "mod_item",
        _ => return None,
    };

    if node.kind() != target_kind {
        return None;
    }

    let has_marker = has_attribute(node, source, marker.marker_name.as_deref()?, marker.marker_argument.as_deref());

    if !has_marker {
        return None;
    }

    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source)?;

    let normalized = normalize_name(&name, framework);

    match marker.creates.as_str() {
        "spec" => Some(SpecNode {
            name: normalized,
            kind: SpecKind::Spec,
            children: vec![],
            line: node.start_position().row + 1,
            parameterized: detect_parameterization(node, source, framework),
            name_is_dynamic: false,
        }),
        "group" => {
            let body = node.child_by_field_name("body")?;
            let children = parse_children_with_shared(body, source, framework, shared);
            Some(SpecNode {
                name: normalized,
                kind: SpecKind::Group,
                children,
                line: node.start_position().row + 1,
                parameterized: None,
                name_is_dynamic: false,
            })
        }
        _ => None,
    }
}

fn has_attribute(node: Node, source: &str, attr_name: &str, attr_argument: Option<&str>) -> bool {
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "attribute_item" {
            if attribute_matches(sib, source, attr_name, attr_argument) {
                return true;
            }
        } else if sib.kind() != "attribute_item" && sib.kind() != "line_comment" && sib.kind() != "block_comment" {
            break;
        }
        sibling = sib.prev_sibling();
    }

    false
}

fn attribute_matches(attr_item: Node, source: &str, attr_name: &str, attr_argument: Option<&str>) -> bool {
    let mut cursor = attr_item.walk();
    for child in attr_item.children(&mut cursor) {
        if child.kind() == "attribute" {
            let ident = child.child_by_field_name("path")
                .or_else(|| {
                    let mut c = child.walk();
                    child.children(&mut c).find(|n| n.kind() == "identifier")
                });
            let Some(ident) = ident else { continue };
            if node_text(ident, source).as_deref() != Some(attr_name) {
                continue;
            }

            if let Some(arg) = attr_argument {
                let mut c2 = child.walk();
                let has_arg = child.children(&mut c2).any(|n| {
                    if n.kind() == "token_tree" {
                        let mut c3 = n.walk();
                        return n.children(&mut c3).any(|inner| {
                            inner.kind() == "identifier" && node_text(inner, source).as_deref() == Some(arg)
                        });
                    }
                    false
                });
                return has_arg;
            }

            return true;
        }
    }
    false
}

fn try_match_name_pattern_marker(
    node: Node,
    source: &str,
    marker: &crate::parse::registry::MarkerDef,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<SpecNode> {
    let ast_kinds = framework.ast_kinds.as_ref();
    let target_kind = match marker.applies_to.as_str() {
        "function" => ast_kinds.and_then(|k| k.function.as_deref())?,
        "method" => ast_kinds.and_then(|k| k.method.as_deref().or(k.function.as_deref()))?,
        "class" => ast_kinds.and_then(|k| k.class.as_deref())?,
        _ => return None,
    };

    if node.kind() != target_kind {
        return None;
    }

    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source)?;

    let pattern = marker.pattern.as_deref()?;
    if !matches_pattern(&name, pattern) {
        return None;
    }

    let normalized = normalize_name(&name, framework);

    match marker.creates.as_str() {
        "spec" => {
            let nested = if !framework.nested_discovery.is_empty() {
                let body_node = node.child_by_field_name("body")
                    .or_else(|| {
                        let mut c = node.walk();
                        node.children(&mut c).find(|n| n.kind() == "block" || n.kind() == "statement_block")
                    });
                if let Some(body) = body_node {
                    find_nested_specs(body, source, framework)
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            if nested.is_empty() {
                let parameterized = detect_parameterization(node, source, framework)
                    .or_else(|| detect_table_driven(node, source, framework));
                Some(SpecNode {
                    name: normalized,
                    kind: SpecKind::Spec,
                    children: vec![],
                    line: node.start_position().row + 1,
                    parameterized,
                    name_is_dynamic: false,
                })
            } else {
                Some(SpecNode {
                    name: normalized,
                    kind: SpecKind::Group,
                    children: nested,
                    line: node.start_position().row + 1,
                    parameterized: None,
                    name_is_dynamic: false,
                })
            }
        }
        "group" => {
            let body_node = node.child_by_field_name("body")
                .or_else(|| {
                    let mut c = node.walk();
                    node.children(&mut c).find(|n| n.kind() == "body_statement" || n.kind() == "block")
                });
            let mut children = inherited_specs(node, source, framework, shared);
            if let Some(body) = body_node {
                children.extend(parse_children_with_shared(body, source, framework, shared));
            }
            Some(SpecNode {
                name: normalized,
                kind: SpecKind::Group,
                children,
                line: node.start_position().row + 1,
                parameterized: None,
                name_is_dynamic: false,
            })
        }
        _ => None,
    }
}

fn find_nested_specs(node: Node, source: &str, framework: &FrameworkDef) -> Vec<SpecNode> {
    let mut results = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if let Some(spec) = try_match_nested_call(child, source, framework) {
            results.push(spec);
        } else {
            results.extend(find_nested_specs(child, source, framework));
        }
    }

    results
}

fn try_match_nested_call(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
) -> Option<SpecNode> {
    for nd in &framework.nested_discovery {
        if node.kind() != nd.ast_type {
            continue;
        }

        let func_node = node.child_by_field_name("function")?;
        let func_text = node_text(func_node, source)?;

        let expected = match (&nd.receiver, &nd.method_name) {
            (Some(recv), Some(method)) => format!("{recv}.{method}"),
            _ => continue,
        };

        if func_text != expected {
            continue;
        }

        let args = find_arguments(node)?;
        let first_arg = args.named_child(0)?;
        let name = extract_string_content(first_arg, source)?;
        let name_is_dynamic = string_node_has_interpolation(unwrap_php_argument(first_arg));

        let nested_children = {
            let block = find_block(node);
            if let Some(b) = block {
                find_nested_specs(b, source, framework)
            } else {
                vec![]
            }
        };

        let kind = if nested_children.is_empty() {
            SpecKind::Spec
        } else {
            SpecKind::Group
        };

        return Some(SpecNode {
            name,
            kind,
            children: nested_children,
            line: node.start_position().row + 1,
            parameterized: None,
            name_is_dynamic,
        });
    }

    None
}

fn detect_parameterization(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
) -> Option<crate::parse::ParamInfo> {
    for param_def in &framework.parameterized {
        let count = match param_def.detection.as_str() {
            "attribute" => count_rust_case_attributes(node, source, param_def.attribute_name.as_deref()?),
            "decorator" => count_python_parametrize_cases(node, source, param_def.decorator_name.as_deref()?),
            "method_call" => count_jest_each_cases(node, source, &param_def.method_names),
            "chained_method" => count_pest_chained_dataset(node, source, &param_def.method_names),
            _ => None,
        };
        if let Some(count) = count
            && count > 0
        {
            return Some(crate::parse::ParamInfo {
                case_count: count,
                labels: vec![],
            });
        }
    }
    None
}

fn detect_table_driven(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
) -> Option<crate::parse::ParamInfo> {
    let table = framework.table_driven.as_ref()?;
    if !table.enabled {
        return None;
    }
    let slice_kind = table.slice_pattern.as_deref()?;
    let loop_kind = table.loop_pattern.as_deref()?;
    let run_call = table.run_call.as_deref()?;

    let body = node.child_by_field_name("body")?;

    let mut has_loop_with_run = false;
    let mut total_cases: usize = 0;

    let mut stack = vec![body];
    while let Some(current) = stack.pop() {
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            if child.kind() == slice_kind
                && let Some(literal_value) = child
                    .children(&mut child.walk())
                    .find(|c| c.kind() == "literal_value")
            {
                let mut lc = literal_value.walk();
                let items = literal_value
                    .children(&mut lc)
                    .filter(|c| c.kind() == "literal_element")
                    .count();
                total_cases += items;
            }

            if child.kind() == loop_kind && call_contains(child, source, run_call) {
                has_loop_with_run = true;
            }

            stack.push(child);
        }
    }

    if has_loop_with_run && total_cases > 0 {
        return Some(crate::parse::ParamInfo {
            case_count: total_cases,
            labels: vec![],
        });
    }
    None
}

fn call_contains(node: Node, source: &str, target: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression"
            && let Some(func) = child.child_by_field_name("function")
            && node_text(func, source).as_deref() == Some(target)
        {
            return true;
        }
        if call_contains(child, source, target) {
            return true;
        }
    }
    false
}

fn count_rust_case_attributes(node: Node, source: &str, attribute_name: &str) -> Option<usize> {
    let mut count = 0;
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "attribute_item" {
            if attribute_matches(sib, source, attribute_name, None) {
                count += 1;
            }
        } else if sib.kind() != "line_comment" && sib.kind() != "block_comment" {
            break;
        }
        sibling = sib.prev_sibling();
    }
    if count == 0 { None } else { Some(count) }
}

fn count_python_parametrize_cases(node: Node, source: &str, decorator_name: &str) -> Option<usize> {
    let parent = node.parent()?;
    if parent.kind() != "decorated_definition" {
        return None;
    }
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        if child.kind() != "decorator" {
            continue;
        }
        let mut dc = child.walk();
        let call = child.children(&mut dc).find(|c| c.kind() == "call")?;
        let func = call.child_by_field_name("function")?;
        if node_text(func, source).as_deref() != Some(decorator_name) {
            continue;
        }
        let args = call.child_by_field_name("arguments")?;
        let mut ac = args.walk();
        for arg in args.children(&mut ac) {
            if arg.kind() == "list" {
                let mut lc = arg.walk();
                let items = arg
                    .children(&mut lc)
                    .filter(|c| c.is_named() && c.kind() != "comment")
                    .count();
                if items > 0 {
                    return Some(items);
                }
            }
        }
    }
    None
}

fn count_pest_chained_dataset(node: Node, source: &str, method_names: &[String]) -> Option<usize> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() != "member_call_expression" {
            return None;
        }
        if parent.child_by_field_name("object")?.id() != current.id() {
            return None;
        }
        let name_node = parent.child_by_field_name("name")?;
        let method = node_text(name_node, source)?;
        if method_names.contains(&method)
            && let Some(count) = count_array_argument(parent)
        {
            return Some(count);
        }
        current = parent;
    }
    None
}

fn count_array_argument(call: Node) -> Option<usize> {
    let args = call.child_by_field_name("arguments")?;
    let mut ac = args.walk();
    for arg in args.children(&mut ac) {
        let inner = unwrap_php_argument(arg);
        if inner.kind() == "array_creation_expression" {
            let mut ic = inner.walk();
            let items = inner
                .children(&mut ic)
                .filter(|c| c.kind() == "array_element_initializer")
                .count();
            if items > 0 {
                return Some(items);
            }
        }
    }
    None
}

fn count_jest_each_cases(node: Node, source: &str, method_names: &[String]) -> Option<usize> {
    if node.kind() != "call_expression" {
        return None;
    }
    let function_node = node.child_by_field_name("function")?;
    if function_node.kind() != "call_expression" {
        return None;
    }
    let inner_fn = function_node.child_by_field_name("function")?;
    let method_text = node_text(inner_fn, source)?;
    if !method_names.contains(&method_text) {
        return None;
    }
    let args = function_node.child_by_field_name("arguments")?;
    let mut ac = args.walk();
    for arg in args.children(&mut ac) {
        if arg.kind() == "array" {
            let mut lc = arg.walk();
            let items = arg
                .children(&mut lc)
                .filter(|c| c.is_named())
                .count();
            if items > 0 {
                return Some(items);
            }
        }
    }
    None
}

fn inherited_specs(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Vec<SpecNode> {
    let Some(registry) = shared else { return vec![] };
    let Some(inheritance) = &framework.inheritance else { return vec![] };
    if !inheritance.enabled {
        return vec![];
    }

    let refs = collect_refs(node, source, framework, inheritance);
    let mut specs = Vec::new();
    for r in refs {
        let resolved = registry.resolve_type(&r);
        if !resolved.is_empty() {
            specs.extend(resolved);
            continue;
        }
        for sep in [".", "::"] {
            if let Some(tail) = r.rsplit(sep).next()
                && tail != r
            {
                let tail_resolved = registry.resolve_type(tail);
                if !tail_resolved.is_empty() {
                    specs.extend(tail_resolved);
                    break;
                }
            }
        }
    }
    specs
}

fn collect_refs(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    inheritance: &crate::parse::registry::InheritanceDef,
) -> Vec<String> {
    let mut refs = Vec::new();
    for det in &inheritance.ref_detection {
        match det.strategy.as_str() {
            "superclass_args" => {
                let default_containers = ["argument_list", "superclasses", "base_clause"];
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    let matches_container = if det.container_kinds.is_empty() {
                        default_containers.contains(&child.kind())
                    } else {
                        det.container_kinds.iter().any(|k| k == child.kind())
                    };
                    if matches_container {
                        let mut inner = child.walk();
                        for arg in child.children(&mut inner) {
                            if det.arg_kinds.iter().any(|k| k == arg.kind())
                                && let Some(text) = node_text(arg, source)
                            {
                                refs.push(text);
                            }
                        }
                    }
                }
            }
            "method_call" => {
                let body = find_type_body(node, framework);
                let Some(body) = body else { continue };
                let mut body_cursor = body.walk();
                for child in body.children(&mut body_cursor) {
                    if !is_dsl_call_kind(child.kind()) {
                        continue;
                    }
                    let Some(method) = extract_method_name(child, source) else { continue };
                    if !det.method_names.iter().any(|m| m == &method) {
                        continue;
                    }
                    let Some(args) = find_arguments(child) else { continue };
                    let mut arg_cursor = args.walk();
                    for arg in args.children(&mut arg_cursor) {
                        if det.arg_kinds.iter().any(|k| k == arg.kind())
                            && let Some(name) = node_text(arg, source)
                        {
                            refs.push(name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    refs
}

fn find_type_body<'a>(node: Node<'a>, framework: &FrameworkDef) -> Option<Node<'a>> {
    let ast = framework.ast_kinds.as_ref()?;
    if let Some(field) = &ast.body_field
        && let Some(body) = node.child_by_field_name(field.as_str())
    {
        return Some(body);
    }
    if let Some(child_kind) = &ast.body_child {
        let mut cursor = node.walk();
        return node.children(&mut cursor).find(|c| c.kind() == child_kind.as_str());
    }
    node.child_by_field_name("body")
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_prefix('^') {
        name.starts_with(prefix)
    } else if let Some(suffix) = pattern.strip_suffix('$') {
        name.ends_with(suffix)
    } else {
        name.contains(pattern)
    }
}

fn normalize_name(name: &str, framework: &FrameworkDef) -> String {
    let norm = match &framework.normalization {
        Some(n) => n,
        None => return name.to_string(),
    };

    if norm.raw {
        return name.to_string();
    }

    let mut result = name.to_string();

    if norm.strip_camel_test_prefix && result.starts_with("Test") && result.len() > 4 {
        let after = &result[4..];
        if after.starts_with(char::is_uppercase) {
            result = after.to_string();
        }
    }

    for prefix in &norm.strip_prefixes {
        if let Some(stripped) = result.strip_prefix(prefix.as_str()) {
            result = stripped.to_string();
            break;
        }
    }

    for suffix in &norm.strip_suffixes {
        if let Some(stripped) = result.strip_suffix(suffix.as_str()) {
            result = stripped.to_string();
            break;
        }
    }

    if norm.underscore_to_space {
        result = result.replace('_', " ");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::parse::registry::all_frameworks;

    fn rspec_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "rspec").expect("rspec framework")
    }

    fn rust_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "rust_builtin").expect("rust_builtin framework")
    }

    #[test]
    fn parse_rspec_basic() {
        let source = r#"
RSpec.describe User do
  describe "validations" do
    it "validates email format" do
      expect(true).to be true
    end

    it "requires password" do
      expect(true).to be true
    end
  end

  describe "associations" do
    it "has many posts" do
      expect(true).to be true
    end
  end
end
"#;
        let tree = parse_file(source, "spec/models/user_spec.rb", rspec_framework());
        assert!(tree.is_some());
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "rspec");

        assert_eq!(tree.root.len(), 1);
        let user = &tree.root[0];
        assert_eq!(user.kind, SpecKind::Group);

        assert_eq!(user.children.len(), 2);
        let validations = &user.children[0];
        assert_eq!(validations.name, "validations");
        assert_eq!(validations.children.len(), 2);
        assert_eq!(validations.children[0].name, "validates email format");
        assert_eq!(validations.children[1].name, "requires password");

        let associations = &user.children[1];
        assert_eq!(associations.name, "associations");
        assert_eq!(associations.children.len(), 1);
        assert_eq!(associations.children[0].name, "has many posts");
    }

    #[test]
    fn parse_rspec_interpolated_name_preserves_placeholder_when_expansion_skipped() {
        // B1: interpolation is preserved verbatim when loop expansion can't
        // substitute (non-literal receiver). Ensures we don't collapse
        // "##{action}" to just "#".
        let source = r###"
RSpec.describe User do
  ACTIONS.each do |action|
    describe "##{action}" do
      it "does something" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/models/user_spec.rb", rspec_framework()).expect("parsed");
        let user = &tree.root[0];
        assert_eq!(user.children.len(), 1);
        assert_eq!(
            user.children[0].name, "##{action}",
            "non-literal receiver should leave placeholder untouched"
        );
    }

    #[test]
    fn parse_rspec_loop_expansion_symbol_array() {
        let source = r###"
RSpec.describe User do
  %i[create update].each do |action|
    describe "##{action}" do
      it "permits access" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/models/user_spec.rb", rspec_framework()).expect("parsed");
        let user = &tree.root[0];
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["#create", "#update"]);
        for child in &user.children {
            assert_eq!(child.kind, SpecKind::Group);
            assert_eq!(child.children.len(), 1);
            assert_eq!(child.children[0].name, "permits access");
        }
    }

    #[test]
    fn parse_rspec_loop_expansion_string_array() {
        let source = r###"
RSpec.describe Fruit do
  %w[apple banana].each do |fruit|
    context "when fruit is #{fruit}" do
      it "tastes good" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/models/fruit_spec.rb", rspec_framework()).expect("parsed");
        let user = &tree.root[0];
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["when fruit is apple", "when fruit is banana"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_plain_array_of_symbols() {
        let source = r###"
RSpec.describe Role do
  [:admin, :user, :guest].each do |role|
    describe "as #{role}" do
      it "behaves correctly" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/models/role_spec.rb", rspec_framework()).expect("parsed");
        let user = &tree.root[0];
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["as admin", "as user", "as guest"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_integer_array() {
        let source = r###"
RSpec.describe Counter do
  [1, 2, 3].each do |n|
    it "handles #{n}" do
    end
  end
end
"###;
        let tree = parse_file(source, "spec/models/counter_spec.rb", rspec_framework()).expect("parsed");
        let user = &tree.root[0];
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["handles 1", "handles 2", "handles 3"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_skipped_for_complex_interpolation() {
        // Complex interpolation #{action.to_sym} can't be statically resolved.
        // Substitution leaves the expression untouched; we still emit one spec
        // per loop iteration with the placeholder partially substituted only
        // where it's an exact #{var} match.
        let source = r###"
RSpec.describe X do
  %i[a b].each do |action|
    describe "for #{action.to_sym}" do
      it "tests" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/x_spec.rb", rspec_framework()).expect("parsed");
        let user = &tree.root[0];
        // Both iterations emit; the placeholder doesn't match the complex
        // expression so the original source survives unchanged.
        assert_eq!(user.children.len(), 2);
        for child in &user.children {
            assert_eq!(child.name, "for #{action.to_sym}");
        }
    }

    #[test]
    fn parse_rspec_loop_expansion_nested_each_produces_cartesian_product() {
        // M2: nested .each loops compose. Outer expansion calls
        // parse_children_with_shared on the body, which re-enters
        // try_expand_loop for the inner. Substitutions stack: inner first
        // (its own var), then outer rewrites the remaining placeholder.
        let source = r###"
RSpec.describe Combo do
  %i[a b].each do |x|
    %i[c d].each do |y|
      describe "#{x}-#{y}" do
        it "tests" do
        end
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/combo_spec.rb", rspec_framework()).expect("parsed");
        let combo = &tree.root[0];
        let names: Vec<&str> = combo.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a-c", "a-d", "b-c", "b-d"]);
        for child in &combo.children {
            assert_eq!(child.kind, SpecKind::Group);
            assert_eq!(child.children.len(), 1);
            assert_eq!(child.children[0].name, "tests");
        }
    }

    #[test]
    fn parse_rspec_loop_expansion_curly_brace_block() {
        // M3: brace-block syntax `{ |x| ... }` should expand the same as
        // do-end. Block kind differs (`block` vs `do_block`) but the
        // `parameters` and `body` fields are present on both.
        let source = r###"
RSpec.describe Brace do
  %i[one two].each { |n|
    describe "##{n}" do
      it "works" do
      end
    end
  }
end
"###;
        let tree = parse_file(source, "spec/brace_spec.rb", rspec_framework()).expect("parsed");
        let brace = &tree.root[0];
        let names: Vec<&str> = brace.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["#one", "#two"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_empty_literal_emits_zero_specs() {
        // m5: empty literal collection means zero runtime iterations.
        // We must not fall through to the recursion fallback (which
        // would emit one phantom describe with the unsubstituted
        // placeholder).
        let source = r###"
RSpec.describe Empty do
  %i[].each do |x|
    describe "##{x}" do
      it "tests" do
      end
    end
  end

  it "still has a real spec" do
  end
end
"###;
        let tree = parse_file(source, "spec/empty_spec.rb", rspec_framework()).expect("parsed");
        let empty = &tree.root[0];
        let names: Vec<&str> = empty.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["still has a real spec"], "empty .each must produce no phantom describe");
    }

    #[test]
    fn parse_rspec_loop_expansion_hash_with_two_block_params() {
        // M1: |k, v| over a hash literal substitutes both placeholders.
        let source = r###"
RSpec.describe Mapping do
  {alpha: 1, beta: 2}.each do |key, value|
    describe "#{key} maps to #{value}" do
      it "tests" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/mapping_spec.rb", rspec_framework()).expect("parsed");
        let mapping = &tree.root[0];
        let names: Vec<&str> = mapping.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["alpha maps to 1", "beta maps to 2"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_paired_array_with_two_block_params() {
        // M1: nested arrays feeding |a, b| substitute both placeholders.
        let source = r###"
RSpec.describe Pairs do
  [[1, "x"], [2, "y"]].each do |n, s|
    describe "n=#{n} s=#{s}" do
      it "tests" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/pairs_spec.rb", rspec_framework()).expect("parsed");
        let pairs = &tree.root[0];
        let names: Vec<&str> = pairs.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["n=1 s=x", "n=2 s=y"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_no_block_parameters_emits_n_copies() {
        // M4: parameter-less block still expands. The body has no
        // placeholders to substitute, so we get N identical groups.
        let source = r###"
RSpec.describe Counter do
  [1, 2, 3].each do
    describe "static block" do
      it "runs" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/counter_spec.rb", rspec_framework()).expect("parsed");
        let counter = &tree.root[0];
        assert_eq!(counter.children.len(), 3, "param-less .each over a 3-element literal must emit 3 copies");
        for child in &counter.children {
            assert_eq!(child.name, "static block");
        }
    }

    #[test]
    fn parse_rspec_loop_substitution_skips_single_quoted_literal() {
        // m6: single-quoted strings don't actually interpolate, so #{x}
        // appearing inside one is literal text. Substitution must NOT
        // touch it. The describe (double-quoted) IS substituted; the
        // inner it (single-quoted) is left alone.
        let source = r###"
RSpec.describe Quoting do
  %i[a b].each do |x|
    describe "##{x}" do
      it 'literal #{x} text' do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/quoting_spec.rb", rspec_framework()).expect("parsed");
        let quoting = &tree.root[0];
        let group_names: Vec<&str> = quoting.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(group_names, vec!["#a", "#b"]);
        for group in &quoting.children {
            assert_eq!(group.children[0].name, "literal #{x} text", "single-quoted literal must not be substituted");
        }
    }

    #[test]
    fn parse_rspec_loop_expansion_substitutes_outer_scope_constant() {
        // m7: a constant assigned in the enclosing scope is collected by
        // the scope-walk and substituted into expanded names alongside
        // the loop variable.
        let source = r###"
RSpec.describe Scoped do
  PREFIX = "p"
  %i[a b].each do |x|
    describe "#{PREFIX}-#{x}" do
      it "tests" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/scoped_spec.rb", rspec_framework()).expect("parsed");
        let scoped = &tree.root[0];
        let names: Vec<&str> = scoped.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["p-a", "p-b"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_nested_each_with_outer_scope_constant() {
        // Nested loops AND an outer-scope constant. Inner expansion must
        // substitute its own loop var plus the outer scope's PREFIX;
        // outer expansion substitutes its own loop var over what remains.
        let source = r###"
RSpec.describe Combo do
  PREFIX = "p"
  %i[a b].each do |x|
    %i[c d].each do |y|
      describe "#{PREFIX}-#{x}-#{y}" do
        it "tests" do
        end
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/combo_spec.rb", rspec_framework()).expect("parsed");
        let combo = &tree.root[0];
        let names: Vec<&str> = combo.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["p-a-c", "p-a-d", "p-b-c", "p-b-d"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_reassigned_constant_uses_closest() {
        // When a constant is assigned twice, the closer assignment (the
        // one nearest to the each-loop in textual order) wins, matching
        // Ruby runtime semantics.
        let source = r###"
RSpec.describe Reassigned do
  PREFIX = "old"
  PREFIX = "new"
  %i[a b].each do |x|
    describe "#{PREFIX}-#{x}" do
      it "tests" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/reassigned_spec.rb", rspec_framework()).expect("parsed");
        let re = &tree.root[0];
        let names: Vec<&str> = re.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["new-a", "new-b"]);
    }

    #[test]
    fn parse_rspec_loop_expansion_paired_array_arity_mismatch() {
        // Ruby destructures by zip-truncation: when the tuple has more
        // values than block params, extras are discarded; when fewer,
        // missing placeholders survive unsubstituted.
        let source = r###"
RSpec.describe Mismatch do
  [[1, "x", true], [2, "y", false]].each do |n, s|
    describe "n=#{n} s=#{s}" do
      it "tests" do
      end
    end
  end

  [[42]].each do |a, b|
    describe "a=#{a} b=#{b}" do
      it "tests" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/mismatch_spec.rb", rspec_framework()).expect("parsed");
        let mis = &tree.root[0];
        let names: Vec<&str> = mis.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec![
            "n=1 s=x",
            "n=2 s=y",
            "a=42 b=#{b}",
        ]);
    }

    #[test]
    fn parse_rspec_loop_expansion_splat_block_parameter_no_substitution() {
        // `|*args|` splat parameters aren't plain identifiers, so we
        // treat it as the no-parameter path: emit N copies without
        // substitution. Matches M4 behavior.
        let source = r###"
RSpec.describe Splat do
  %i[a b].each do |*args|
    describe "static" do
      it "tests" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "spec/splat_spec.rb", rspec_framework()).expect("parsed");
        let splat = &tree.root[0];
        assert_eq!(splat.children.len(), 2);
        for child in &splat.children {
            assert_eq!(child.name, "static");
        }
    }

    #[test]
    fn parse_minitest_loop_expansion_symbol_array() {
        let minitest = all_frameworks().iter().find(|f| f.name == "minitest").expect("minitest");
        let source = r###"
class TestFoo < Minitest::Test
  %i[one two].each do |n|
    describe "##{n}" do
      it "works" do
      end
    end
  end
end
"###;
        let tree = parse_file(source, "test/test_foo.rb", minitest).expect("parsed");
        let foo_test = &tree.root[0];
        assert_eq!(foo_test.kind, SpecKind::Group);
        let names: Vec<&str> = foo_test.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["#one", "#two"]);
    }

    #[test]
    fn parse_rspec_escape_in_name_returns_full_raw_text() {
        let source = r#"
RSpec.describe User do
  it "handles a \n newline in the name" do
    expect(true).to be true
  end
end
"#;
        let tree = parse_file(source, "spec/models/user_spec.rb", rspec_framework()).expect("parsed");
        let user = &tree.root[0];
        assert_eq!(user.children.len(), 1);
        assert_eq!(
            user.children[0].name,
            r#"handles a \n newline in the name"#,
            "multi-chunk Ruby strings must return the full raw text, not just the first string_content child"
        );
    }

    #[test]
    fn parse_rspec_with_context() {
        let source = r#"
RSpec.describe "API" do
  context "when authenticated" do
    it "returns 200" do
    end
  end

  context "when unauthenticated" do
    it "returns 401" do
    end
  end
end
"#;
        let tree = parse_file(source, "spec/requests/api_spec.rb", rspec_framework());
        let tree = tree.expect("parsed");
        let api = &tree.root[0];
        assert_eq!(api.name, "API");
        assert_eq!(api.children.len(), 2);
        assert_eq!(api.children[0].name, "when authenticated");
        assert_eq!(api.children[1].name, "when unauthenticated");
    }

    #[test]
    fn parse_rust_test_functions() {
        let source = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn test_addition() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn test_subtraction() {
        assert_eq!(5 - 3, 2);
    }
}
"#;
        let tree = parse_file(source, "src/lib.rs", rust_framework());
        assert!(tree.is_some());
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "rust_builtin");

        assert_eq!(tree.root.len(), 1);
        let tests_mod = &tree.root[0];
        assert_eq!(tests_mod.name, "tests");
        assert_eq!(tests_mod.kind, SpecKind::Group);

        assert_eq!(tests_mod.children.len(), 2);
        assert_eq!(tests_mod.children[0].name, "addition");
        assert_eq!(tests_mod.children[0].kind, SpecKind::Spec);
        assert_eq!(tests_mod.children[1].name, "subtraction");
    }

    #[test]
    fn parse_rust_without_module() {
        let source = r#"
#[test]
fn test_standalone() {
    assert!(true);
}
"#;
        let tree = parse_file(source, "tests/basic.rs", rust_framework());
        assert!(tree.is_some());
        let tree = tree.expect("parsed");
        assert_eq!(tree.root.len(), 1);
        assert_eq!(tree.root[0].name, "standalone");
        assert_eq!(tree.root[0].kind, SpecKind::Spec);
    }

    fn read_fixture(rel_path: &str) -> Option<String> {
        let fixtures_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../specdiff-tests/fixtures");
        let fixtures_path = std::path::Path::new(fixtures_dir);
        if !fixtures_path.exists() {
            eprintln!("skipping fixture test: specdiff-tests repo not found at {fixtures_dir}");
            return None;
        }
        let full = fixtures_path.join(rel_path);
        Some(std::fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("fixture file {} should be readable: {e}", full.display())))
    }

    #[test]
    fn parse_rspec_fixture_base() {
        let Some(source) = read_fixture("rspec/base/spec/models/user_spec.rb") else { return };
        let tree = parse_file(&source, "spec/models/user_spec.rb", rspec_framework());
        let tree = tree.expect("parsed fixture");
        let user = &tree.root[0];
        assert_eq!(user.children.len(), 2);
    }

    #[test]
    fn parse_rust_fixture_base() {
        let Some(source) = read_fixture("rust_builtin/base/src/lib.rs") else { return };
        let tree = parse_file(&source, "src/lib.rs", rust_framework());
        let tree = tree.expect("parsed fixture");
        assert_eq!(tree.root.len(), 1);
        let tests_mod = &tree.root[0];
        assert_eq!(tests_mod.children.len(), 3);
    }

    #[test]
    fn normalize_rust_test_names() {
        let fw = rust_framework();
        assert_eq!(normalize_name("test_addition", fw), "addition");
        assert_eq!(normalize_name("test_my_func", fw), "my func");
        assert_eq!(normalize_name("helper_func", fw), "helper func");
    }

    #[test]
    fn rust_attribute_no_false_positives() {
        let source = r#"
#[testing_helper]
fn setup_testing() {}

#[test]
fn test_real() {
    assert!(true);
}
"#;
        let tree = parse_file(source, "tests/basic.rs", rust_framework());
        let tree = tree.expect("parsed");
        assert_eq!(tree.root.len(), 1, "should only find #[test], not #[testing_helper]");
        assert_eq!(tree.root[0].name, "real");
    }

    fn minitest_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "minitest").expect("minitest framework")
    }

    #[test]
    fn parse_minitest_class_and_methods() {
        let source = r#"
require "test_helper"

class TestUser < Minitest::Test
  def test_valid_user
    assert true
  end

  def test_invalid_without_name
    refute false
  end
end
"#;
        let tree = parse_file(source, "test/models/user_test.rb", minitest_framework());
        assert!(tree.is_some());
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "minitest");

        assert_eq!(tree.root.len(), 1);
        let user_class = &tree.root[0];
        assert_eq!(user_class.name, "User");
        assert_eq!(user_class.kind, SpecKind::Group);
        assert_eq!(user_class.children.len(), 2);
        assert_eq!(user_class.children[0].name, "valid user");
        assert_eq!(user_class.children[1].name, "invalid without name");
    }

    #[test]
    fn parse_minitest_fixture_base() {
        let Some(source) = read_fixture("minitest/base/test/models/user_test.rb") else { return };
        let tree = parse_file(&source, "test/models/user_test.rb", minitest_framework());
        let tree = tree.expect("parsed fixture");
        assert_eq!(tree.root.len(), 1);
        let user_class = &tree.root[0];
        assert_eq!(user_class.children.len(), 3);
    }

    #[test]
    fn parse_empty_source_returns_none() {
        assert!(parse_file("", "spec/empty_spec.rb", rspec_framework()).is_none());
        assert!(parse_file("", "src/lib.rs", rust_framework()).is_none());
    }

    #[test]
    fn parse_source_with_no_tests_returns_none() {
        let source = "class User\n  def name\n    @name\n  end\nend\n";
        assert!(parse_file(source, "spec/models/user_spec.rb", rspec_framework()).is_none());
    }

    fn pytest_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "pytest").expect("pytest framework")
    }

    fn jest_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "jest").expect("jest framework")
    }

    fn go_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "go_testing").expect("go_testing framework")
    }

    #[test]
    fn parse_pytest_class_and_functions() {
        let source = "class TestUser:\n    def test_create(self):\n        assert True\n\n    def test_delete(self):\n        assert True\n\ndef test_standalone():\n    assert True\n";
        let tree = parse_file(source, "tests/test_user.py", pytest_framework());
        assert!(tree.is_some(), "should parse pytest");
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "pytest");

        assert!(tree.root.len() >= 2, "should find class + standalone fn, got {}", tree.root.len());

        let class = tree.root.iter().find(|n| n.kind == SpecKind::Group);
        assert!(class.is_some(), "should find TestUser group");
        let class = class.expect("found");
        assert_eq!(class.name, "User");
        assert_eq!(class.children.len(), 2);
        assert_eq!(class.children[0].name, "create");
        assert_eq!(class.children[1].name, "delete");

        let standalone = tree.root.iter().find(|n| n.name == "standalone");
        assert!(standalone.is_some(), "should find test_standalone");
    }

    #[test]
    fn parse_pytest_fixture() {
        let Some(source) = read_fixture("pytest/base/tests/test_user.py") else { return };
        let tree = parse_file(&source, "tests/test_user.py", pytest_framework());
        let tree = tree.expect("parsed fixture");
        assert!(tree.root.len() >= 2, "should have TestUser + TestUserValidation");
    }

    #[test]
    fn parse_jest_describe_it() {
        let source = "describe('User', () => {\n  it('creates a user', () => {\n    expect(true).toBe(true);\n  });\n\n  it('deletes a user', () => {\n    expect(true).toBe(true);\n  });\n});\n";
        let tree = parse_file(source, "user.test.js", jest_framework());
        assert!(tree.is_some(), "should parse jest");
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "jest");
        assert_eq!(tree.root.len(), 1);

        let describe = &tree.root[0];
        assert_eq!(describe.name, "User");
        assert_eq!(describe.kind, SpecKind::Group);
        assert_eq!(describe.children.len(), 2);
        assert_eq!(describe.children[0].name, "creates a user");
        assert_eq!(describe.children[1].name, "deletes a user");
    }

    #[test]
    fn parse_jest_fixture() {
        let Some(source) = read_fixture("jest/base/__tests__/user.test.js") else { return };
        let tree = parse_file(&source, "__tests__/user.test.js", jest_framework());
        let tree = tree.expect("parsed fixture");
        assert_eq!(tree.root.len(), 1);
        let user = &tree.root[0];
        assert_eq!(user.name, "User");
        assert_eq!(user.children.len(), 2);
    }

    #[test]
    fn parse_go_test_functions() {
        let source = "package user\n\nimport \"testing\"\n\nfunc TestCreate(t *testing.T) {\n}\n\nfunc TestDelete(t *testing.T) {\n}\n";
        let tree = parse_file(source, "user_test.go", go_framework());
        assert!(tree.is_some(), "should parse go tests");
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "go_testing");
        assert_eq!(tree.root.len(), 2);
        assert_eq!(tree.root[0].name, "Create");
        assert_eq!(tree.root[1].name, "Delete");
    }

    #[test]
    fn parse_go_t_run_subtests() {
        let source = "package user\n\nimport \"testing\"\n\nfunc TestCreate(t *testing.T) {\n\tt.Run(\"with valid name\", func(t *testing.T) {})\n\tt.Run(\"with valid email\", func(t *testing.T) {})\n}\n";
        let tree = parse_file(source, "user_test.go", go_framework());
        let tree = tree.expect("parsed");
        assert_eq!(tree.root.len(), 1);

        let create = &tree.root[0];
        assert_eq!(create.name, "Create");
        assert_eq!(create.kind, SpecKind::Group);
        assert_eq!(create.children.len(), 2);
        assert_eq!(create.children[0].name, "with valid name");
        assert_eq!(create.children[1].name, "with valid email");
    }

    #[test]
    fn parse_go_fixture() {
        let Some(source) = read_fixture("go/base/user_test.go") else { return };
        let tree = parse_file(&source, "user_test.go", go_framework());
        let tree = tree.expect("parsed fixture");
        assert_eq!(tree.root.len(), 2);

        let create = &tree.root[0];
        assert_eq!(create.name, "CreateUser");
        assert_eq!(create.kind, SpecKind::Group);
        assert_eq!(create.children.len(), 2, "should find 2 t.Run subtests");
    }

    fn exunit_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "exunit").expect("exunit")
    }

    #[test]
    fn parse_exunit_describe_test() {
        let source = r#"defmodule UserTest do
  use ExUnit.Case

  describe "validations" do
    test "validates email" do
      assert true
    end

    test "requires name" do
      assert true
    end
  end

  test "standalone test" do
    assert true
  end
end
"#;
        let tree = parse_file(source, "test/user_test.exs", exunit_framework());
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "exunit");

        let validations = tree.root.iter().find(|n| n.name == "validations").expect("validations");
        assert_eq!(validations.kind, SpecKind::Group);
        assert_eq!(validations.children.len(), 2);
        assert_eq!(validations.children[0].name, "validates email");
        assert_eq!(validations.children[1].name, "requires name");

        assert!(tree.root.iter().any(|n| n.name == "standalone test" && n.kind == SpecKind::Spec));
    }

    #[test]
    fn parse_exunit_fixture() {
        let Some(source) = read_fixture("exunit/base/test/user_test.exs") else { return };
        let tree = parse_file(&source, "test/user_test.exs", exunit_framework());
        let tree = tree.expect("parsed fixture");
        let validations = tree.root.iter().find(|n| n.name == "validations").expect("validations");
        assert_eq!(validations.children.len(), 2);
    }

    #[test]
    fn pytest_class_inheritance_inlines_parent_methods() {
        let source = r#"
class BaseTest:
    def test_shared_one(self):
        pass

    def test_shared_two(self):
        pass

class TestUser(BaseTest):
    def test_own(self):
        pass
"#;

        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(source, pytest_framework(), &mut registry);

        let tree = parse_file_with_shared(source, "tests/test_user.py", pytest_framework(), Some(&registry));
        let tree = tree.expect("parsed");

        let test_user = tree.root.iter().find(|n| n.name == "User").expect("TestUser -> User");
        let names: Vec<&str> = test_user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"shared one"), "inherited test_shared_one should appear, got {names:?}");
        assert!(names.contains(&"shared two"));
        assert!(names.contains(&"own"));
    }

    #[test]
    fn parse_rstest_case_count() {
        let source = r#"
#[cfg(test)]
mod tests {
    #[rstest]
    #[case(1, 2)]
    #[case(3, 4)]
    #[case(5, 6)]
    fn test_cases(#[case] a: i32, #[case] b: i32) {
    }
}
"#;
        let tree = parse_file(source, "src/lib.rs", rust_framework()).expect("parsed");
        let tests = &tree.root[0];
        let cases = tests.children.iter().find(|n| n.name == "cases").expect("cases");
        let param = cases.parameterized.as_ref().expect("has parameterization");
        assert_eq!(param.case_count, 3);
    }

    #[test]
    fn parse_pytest_parametrize_count() {
        let source = r#"
import pytest

@pytest.mark.parametrize("a,b", [(1, 2), (3, 4), (5, 6), (7, 8)])
def test_add(a, b):
    assert a + b > 0
"#;
        let tree = parse_file(source, "tests/test_math.py", pytest_framework()).expect("parsed");
        let add = tree.root.iter().find(|n| n.name == "add").expect("add");
        let param = add.parameterized.as_ref().expect("has parameterization");
        assert_eq!(param.case_count, 4);
    }

    #[test]
    fn parse_go_table_driven() {
        let source = r#"package user
import "testing"

func TestAdd(t *testing.T) {
    cases := []struct{
        name string
        a int
    }{
        {"zero", 0},
        {"one", 1},
        {"two", 2},
        {"three", 3},
    }
    for _, tc := range cases {
        t.Run(tc.name, func(t *testing.T) {})
    }
}
"#;
        let tree = parse_file(source, "user_test.go", go_framework()).expect("parsed");
        let add = tree.root.iter().find(|n| n.name == "Add").expect("Add");
        let param = add.parameterized.as_ref().expect("table-driven detected");
        assert_eq!(param.case_count, 4);
    }

    #[test]
    fn parse_jest_each_count() {
        let source = "it.each([[1,2],[3,4],[5,6]])('adds %i + %i', (a,b) => {});\n";
        let tree = parse_file(source, "user.test.js", jest_framework()).expect("parsed");
        assert_eq!(tree.root.len(), 1);
        let each = &tree.root[0];
        let param = each.parameterized.as_ref().expect("has parameterization");
        assert_eq!(param.case_count, 3);
    }

    #[test]
    fn minitest_module_include_inlines_methods() {
        let source = r#"
module SharedBehavior
  def test_shared_one
    assert true
  end

  def test_shared_two
    assert true
  end
end

class TestUser < Minitest::Test
  include SharedBehavior

  def test_own
    assert true
  end
end
"#;

        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(source, minitest_framework(), &mut registry);

        let tree = parse_file_with_shared(source, "test/user_test.rb", minitest_framework(), Some(&registry));
        let tree = tree.expect("parsed");

        let test_user = tree.root.iter().find(|n| n.name == "User").expect("TestUser -> User");
        let names: Vec<&str> = test_user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"shared one"), "included test_shared_one should appear, got {names:?}");
        assert!(names.contains(&"shared two"));
        assert!(names.contains(&"own"));
    }

    #[test]
    fn pytest_inheritance_resolves_dotted_base() {
        let parent = r#"
class TestPersistable:
    def test_has_id(self):
        pass
"#;
        let child = r#"
import mixins

class TestUser(mixins.TestPersistable):
    def test_own(self):
        pass
"#;
        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(parent, pytest_framework(), &mut registry);

        let tree = parse_file_with_shared(child, "tests/test_user.py", pytest_framework(), Some(&registry));
        let tree = tree.expect("parsed");
        let user = tree.root.iter().find(|n| n.name == "User").expect("User");
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"has id"), "mixins.TestPersistable base should resolve, got {names:?}");
        assert!(names.contains(&"own"));
    }

    #[test]
    fn ruby_scope_resolution_module_name() {
        let support = r#"
module Foo
  module SharedBehavior
    def test_shared
      assert true
    end
  end
end
"#;
        let test = r#"
class TestUser < Minitest::Test
  include Foo::SharedBehavior
  def test_own
    assert true
  end
end
"#;
        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(support, minitest_framework(), &mut registry);

        let tree = parse_file_with_shared(test, "test/user_test.rb", minitest_framework(), Some(&registry));
        let tree = tree.expect("parsed");
        let user = tree.root.iter().find(|n| n.name == "User").expect("User");
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"shared"), "Foo::SharedBehavior should resolve, got {names:?}");
        assert!(names.contains(&"own"));
    }

    #[test]
    fn ruby_leaf_name_include_resolves() {
        let support = r#"
module Support
  module Persistable
    def test_has_id
      assert true
    end
  end
end
"#;
        let test = r#"
class TestUser < Minitest::Test
  include Persistable
  def test_own
    assert true
  end
end
"#;
        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(support, minitest_framework(), &mut registry);

        let tree = parse_file_with_shared(test, "test/user_test.rb", minitest_framework(), Some(&registry));
        let tree = tree.expect("parsed");
        let user = tree.root.iter().find(|n| n.name == "User").expect("User");
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"has id"), "bare leaf include Persistable should resolve, got {names:?}");
    }

    #[test]
    fn ruby_transitive_module_include() {
        let source = r#"
module Inner
  def test_deep
    assert true
  end
end

module Outer
  include Inner

  def test_outer
    assert true
  end
end

class TestUser < Minitest::Test
  include Outer

  def test_own
    assert true
  end
end
"#;
        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(source, minitest_framework(), &mut registry);

        let tree = parse_file_with_shared(source, "test/user_test.rb", minitest_framework(), Some(&registry));
        let tree = tree.expect("parsed");
        let user = tree.root.iter().find(|n| n.name == "User").expect("User");
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"deep"), "transitive include should surface Inner's test, got {names:?}");
        assert!(names.contains(&"outer"));
        assert!(names.contains(&"own"));
    }

    #[test]
    fn go_table_driven_multiple_slices_sums_counts() {
        let source = r#"package user
import "testing"

func TestValidate(t *testing.T) {
    emails := []struct{ name, input string }{
        {"valid", "a@b"},
        {"empty", ""},
    }
    for _, tc := range emails {
        t.Run(tc.name, func(t *testing.T) { _ = tc.input })
    }

    numbers := []struct{ name string; n int }{
        {"zero", 0},
        {"one", 1},
        {"two", 2},
    }
    for _, tc := range numbers {
        t.Run(tc.name, func(t *testing.T) { _ = tc.n })
    }
}
"#;
        let tree = parse_file(source, "user_test.go", go_framework()).expect("parsed");
        let v = tree.root.iter().find(|n| n.name == "Validate").expect("Validate");
        let param = v.parameterized.as_ref().expect("has parameterization");
        assert_eq!(param.case_count, 5, "should sum both slices: 2 + 3");
    }

    fn phpunit_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "phpunit").expect("phpunit")
    }

    #[test]
    fn parse_phpunit_class_and_methods() {
        let source = "<?php\nclass UserTest extends TestCase {\n    public function testCreate(): void {\n        $this->assertTrue(true);\n    }\n    public function testDelete(): void {\n        $this->assertTrue(true);\n    }\n    public function helperMethod(): void {\n    }\n}\n";
        let tree = parse_file(source, "tests/UserTest.php", phpunit_framework());
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "phpunit");
        assert_eq!(tree.root.len(), 1);

        let user = &tree.root[0];
        assert_eq!(user.name, "User");
        assert_eq!(user.kind, SpecKind::Group);
        assert_eq!(user.children.len(), 2, "should find testCreate + testDelete, not helperMethod");
        assert_eq!(user.children[0].name, "Create");
        assert_eq!(user.children[1].name, "Delete");
    }

    #[test]
    fn parse_phpunit_class_inheritance() {
        let source = "<?php\nclass BaseTestCase extends TestCase {\n    public function testShared(): void {\n        $this->assertTrue(true);\n    }\n}\n\nclass UserTest extends BaseTestCase {\n    public function testOwn(): void {\n        $this->assertTrue(true);\n    }\n}\n";
        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(source, phpunit_framework(), &mut registry);

        let tree = parse_file_with_shared(source, "tests/UserTest.php", phpunit_framework(), Some(&registry));
        let tree = tree.expect("parsed");

        let user = tree.root.iter().find(|n| n.name == "User").expect("UserTest -> User");
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Shared"), "inherited testShared should appear, got {names:?}");
        assert!(names.contains(&"Own"));
    }

    fn junit_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "junit").expect("junit")
    }

    #[test]
    fn parse_junit_annotated_methods() {
        let source = "class UserTest {\n    @Test\n    void testCreate() {\n    }\n    @Test\n    void testDelete() {\n    }\n    void helperMethod() {\n    }\n}\n";
        let tree = parse_file(source, "tests/UserTest.java", junit_framework());
        let tree = tree.expect("parsed");
        assert_eq!(tree.framework, "junit");
        assert_eq!(tree.root.len(), 1);

        let user = &tree.root[0];
        assert_eq!(user.name, "User");
        assert_eq!(user.kind, SpecKind::Group);
        assert_eq!(user.children.len(), 2, "should find @Test methods only, not helperMethod");
        assert_eq!(user.children[0].name, "Create");
        assert_eq!(user.children[1].name, "Delete");
    }

    #[test]
    fn parse_junit_class_inheritance() {
        let source = "class BaseTest {\n    @Test\n    void testShared() {\n    }\n}\n\nclass UserTest extends BaseTest {\n    @Test\n    void testOwn() {\n    }\n}\n";
        let mut registry = crate::parse::shared::SharedExampleRegistry::default();
        crate::parse::shared::scan_for_definitions(source, junit_framework(), &mut registry);

        let tree = parse_file_with_shared(source, "tests/UserTest.java", junit_framework(), Some(&registry));
        let tree = tree.expect("parsed");

        let user = tree.root.iter().find(|n| n.name == "User").expect("UserTest -> User");
        let names: Vec<&str> = user.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Shared"), "inherited testShared should appear, got {names:?}");
        assert!(names.contains(&"Own"));
    }

    #[test]
    fn parse_junit_nested_class() {
        let source = "class UserTest {\n    @Test\n    void testCreate() {\n    }\n\n    @Nested\n    class AdminTest {\n        @Test\n        void testAdmin() {\n        }\n    }\n}\n";
        let tree = parse_file(source, "tests/UserTest.java", junit_framework());
        let tree = tree.expect("parsed");

        let user = &tree.root[0];
        assert_eq!(user.name, "User");
        assert_eq!(user.children.len(), 2);

        let create = &user.children[0];
        assert_eq!(create.name, "Create");
        assert_eq!(create.kind, SpecKind::Spec);

        let admin = &user.children[1];
        assert_eq!(admin.name, "Admin");
        assert_eq!(admin.kind, SpecKind::Group);
        assert_eq!(admin.children.len(), 1);
        assert_eq!(admin.children[0].name, "Admin");
    }

    fn pest_framework() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "pest").expect("pest")
    }

    #[test]
    fn parse_pest_top_level_test_calls() {
        let source = "<?php\n\ntest('software config loads', function () {\n    assert_true(true);\n});\n\nit('handles empty body', function () {\n    assert_eq(0, 0);\n});\n\nhelperFunction();\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        assert_eq!(tree.framework, "pest");
        let names: Vec<&str> = tree.root.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["software config loads", "handles empty body"]);
        for node in &tree.root {
            assert_eq!(node.kind, SpecKind::Spec);
        }
    }

    #[test]
    fn parse_pest_double_quoted_names() {
        let source = "<?php\n\ntest(\"double quoted spec\", function () {});\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        assert_eq!(tree.root.len(), 1);
        assert_eq!(tree.root[0].name, "double quoted spec");
    }

    #[test]
    fn parse_pest_describe_groups_nested_tests() {
        let source = "<?php\n\ndescribe('User', function () {\n    test('is created', function () {});\n    it('can be deleted', function () {});\n});\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        assert_eq!(tree.root.len(), 1);
        let user = &tree.root[0];
        assert_eq!(user.name, "User");
        assert_eq!(user.kind, SpecKind::Group);
        let child_names: Vec<&str> = user.children.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(child_names, vec!["is created", "can be deleted"]);
    }

    #[test]
    fn parse_pest_ignores_non_test_function_calls() {
        let source = "<?php\n\nrequire_once 'helpers.php';\nsome_helper('not a test');\ntest('real test', function () {});\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        assert_eq!(tree.root.len(), 1);
        assert_eq!(tree.root[0].name, "real test");
    }

    #[test]
    fn parse_pest_ignores_lifecycle_hooks_and_uses() {
        let source = "<?php\n\nuses(TestCase::class)->in('Feature');\nbeforeEach(function () {});\nafterEach(function () {});\nbeforeAll(function () {});\nafterAll(function () {});\ntest('only real spec', function () {});\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        let names: Vec<&str> = tree.root.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["only real spec"], "hooks and uses must not become specs");
    }

    #[test]
    fn parse_pest_preserves_interpolated_test_name() {
        let source = "<?php\n\ntest(\"user $name is created\", function () {});\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        assert_eq!(tree.root.len(), 1);
        assert_eq!(tree.root[0].name, "user $name is created");
    }

    #[test]
    fn parse_pest_chained_modifier_still_yields_spec() {
        let source = "<?php\n\ntest('x', function () {})->skip();\ntest('y', function () {})->throws(Exception::class);\ntest('z', function () {})->group('api')->skip();\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        let names: Vec<&str> = tree.root.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["x", "y", "z"]);
    }

    #[test]
    fn parse_pest_with_dataset_counts_cases() {
        let source = "<?php\n\ntest('validates email', function ($email) {})->with(['a@b.com', 'c@d.com', 'e@f.com']);\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        assert_eq!(tree.root.len(), 1);
        let spec = &tree.root[0];
        assert_eq!(spec.name, "validates email");
        let param = spec.parameterized.as_ref().expect("parameterized info");
        assert_eq!(param.case_count, 3);
    }

    #[test]
    fn parse_pest_with_dataset_survives_chained_skip() {
        let source = "<?php\n\ntest('validates email', fn ($e) => $e)->with(['a@b.com', 'c@d.com'])->skip();\n";
        let tree = parse_file(source, "tests/run.php", pest_framework()).expect("parsed");
        let param = tree.root[0].parameterized.as_ref().expect("parameterized info");
        assert_eq!(param.case_count, 2);
    }
}
