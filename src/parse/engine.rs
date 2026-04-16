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
        } else {
            results.extend(parse_children_with_shared(child, source, framework, shared));
        }
    }

    results
}

fn is_inert_container(node: Node, framework: &FrameworkDef) -> bool {
    matches!(
        (framework.language.as_str(), node.kind()),
        ("ruby", "module")
    )
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

fn try_match_inclusion(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
    shared: Option<&SharedExampleRegistry>,
) -> Option<Vec<SpecNode>> {
    let shared_def = framework.shared.as_ref()?;
    let registry = shared?;

    if node.kind() != "call" && node.kind() != "call_expression" {
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
    if node.kind() != "call" && node.kind() != "call_expression" {
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
                });
            }
        }
    }

    None
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
                let object = function_node.child_by_field_name("object")?;
                return node_text(object, source);
            }
            node_text(function_node, source)
        }
        _ => None,
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
            let first_arg = args.named_child(0)?;
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
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_string_content(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" | "string_literal" | "interpreted_string_literal" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "string_content"
                    | "string_fragment"
                    | "interpreted_string_literal_content"
                    | "quoted_content" => {
                        return node_text(child, source);
                    }
                    _ => {}
                }
            }
            let text = node_text(node, source)?;
            Some(text.trim_matches(|c| c == '"' || c == '\'').to_string())
        }
        _ => None,
    }
}

fn node_text(node: Node, source: &str) -> Option<String> {
    source.get(node.byte_range()).map(|s| s.to_string())
}

pub fn node_text_pub(node: Node, source: &str) -> Option<String> {
    node_text(node, source)
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
            _ => {}
        }
    }
    None
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
    let target_kind = match marker.applies_to.as_str() {
        "function" | "method" => match framework.language.as_str() {
            "ruby" => "method",
            "python" => "function_definition",
            "go" => "function_declaration",
            _ => return None,
        },
        "class" => match framework.language.as_str() {
            "ruby" => "class",
            "python" => "class_definition",
            _ => return None,
        },
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
                })
            } else {
                Some(SpecNode {
                    name: normalized,
                    kind: SpecKind::Group,
                    children: nested,
                    line: node.start_position().row + 1,
                    parameterized: None,
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
    let mut case_count: Option<usize> = None;

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
                if items > 0 && case_count.is_none() {
                    case_count = Some(items);
                }
            }

            if child.kind() == loop_kind && call_contains(child, source, run_call) {
                has_loop_with_run = true;
            }

            stack.push(child);
        }
    }

    if has_loop_with_run
        && let Some(count) = case_count
    {
        return Some(crate::parse::ParamInfo {
            case_count: count,
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

    match framework.language.as_str() {
        "python" => python_inherited(node, source, registry),
        "ruby" => ruby_included(node, source, registry),
        _ => vec![],
    }
}

fn python_inherited(
    node: Node,
    source: &str,
    registry: &SharedExampleRegistry,
) -> Vec<SpecNode> {
    let mut specs = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "argument_list" {
            let mut inner = child.walk();
            for arg in child.children(&mut inner) {
                if arg.kind() == "identifier"
                    && let Some(parent_name) = node_text(arg, source)
                    && let Some(parent_specs) = registry.get_type(&parent_name)
                {
                    specs.extend_from_slice(parent_specs);
                }
            }
        }
    }
    specs
}

fn ruby_included(
    node: Node,
    source: &str,
    registry: &SharedExampleRegistry,
) -> Vec<SpecNode> {
    let mut specs = Vec::new();
    let mut cursor = node.walk();
    let body = node.children(&mut cursor).find(|c| c.kind() == "body_statement");
    let Some(body) = body else { return specs };

    let mut body_cursor = body.walk();
    for child in body.children(&mut body_cursor) {
        if child.kind() != "call" {
            continue;
        }
        let Some(method) = extract_method_name(child, source) else { continue };
        if method != "include" && method != "extend" {
            continue;
        }

        let Some(args) = find_arguments(child) else { continue };
        let mut arg_cursor = args.walk();
        for arg in args.children(&mut arg_cursor) {
            if (arg.kind() == "constant" || arg.kind() == "scope_resolution")
                && let Some(name) = node_text(arg, source)
                && let Some(module_specs) = registry.get_type(&name)
            {
                specs.extend_from_slice(module_specs);
            }
        }
    }
    specs
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_prefix('^') {
        name.starts_with(prefix)
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
}
