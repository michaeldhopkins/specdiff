use crate::parse::registry::FrameworkDef;
use crate::parse::{SpecKind, SpecNode, SpecTree};
use tree_sitter::{Node, Parser};

pub fn parse_file(source: &str, path: &str, framework: &FrameworkDef) -> Option<SpecTree> {
    let mut parser = Parser::new();
    let language = language_for_framework(framework)?;
    parser.set_language(&language).ok()?;

    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let children = parse_children(root, source, framework);

    if children.is_empty() {
        return None;
    }

    Some(SpecTree {
        file_path: path.to_string(),
        framework: framework.name.clone(),
        root: children,
    })
}

fn language_for_framework(framework: &FrameworkDef) -> Option<tree_sitter::Language> {
    match framework.language.as_str() {
        "ruby" => Some(tree_sitter_ruby::LANGUAGE.into()),
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        _ => None,
    }
}

fn parse_children(node: Node, source: &str, framework: &FrameworkDef) -> Vec<SpecNode> {
    let mut results = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if let Some(spec_node) = try_match_node(child, source, framework) {
            results.push(spec_node);
        } else {
            results.extend(parse_children(child, source, framework));
        }
    }

    results
}

fn try_match_node(node: Node, source: &str, framework: &FrameworkDef) -> Option<SpecNode> {
    if let Some(result) = try_match_dsl_node(node, source, framework) {
        return Some(result);
    }
    if let Some(result) = try_match_marker_node(node, source, framework) {
        return Some(result);
    }
    None
}

fn try_match_dsl_node(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
) -> Option<SpecNode> {
    if node.kind() != "call" && node.kind() != "call_expression" {
        return None;
    }

    let method_name = extract_method_name(node, source)?;

    for group_def in &framework.group {
        if group_def.ast_type == node.kind() && group_def.method_names.contains(&method_name) {
            if let Some(name) = extract_name(node, source, &group_def.name_source, group_def.name_source_type.as_deref()) {
                let block_node = find_block(node);
                let children = if let Some(block) = block_node {
                    parse_children(block, source, framework)
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
                    parameterized: None,
                });
            }
        }
    }

    None
}

fn extract_method_name(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "call" => {
            let method_node = node.child_by_field_name("method")?;
            Some(node_text(method_node, source))
        }
        "call_expression" => {
            let function_node = node.child_by_field_name("function")?;
            Some(node_text(function_node, source))
        }
        _ => None,
    }
}

fn extract_name(
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
                Some("constant") => Some(node_text(first_arg, source)),
                _ => Some(node_text(first_arg, source)),
            }
        }
        "identifier" => {
            let name_node = node.child_by_field_name("name")?;
            Some(node_text(name_node, source))
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

fn find_block(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "block" | "do_block" => return Some(child),
            _ => {}
        }
    }
    None
}

fn extract_string_content(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" | "string_literal" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "string_content" {
                    return Some(node_text(child, source));
                }
            }
            let text = node_text(node, source);
            let trimmed = text
                .strip_prefix('"')
                .or_else(|| text.strip_prefix('\''))
                .unwrap_or(&text);
            let trimmed = trimmed
                .strip_suffix('"')
                .or_else(|| trimmed.strip_suffix('\''))
                .unwrap_or(trimmed);
            Some(trimmed.to_string())
        }
        _ => None,
    }
}

fn node_text(node: Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn try_match_marker_node(
    node: Node,
    source: &str,
    framework: &FrameworkDef,
) -> Option<SpecNode> {
    for marker in &framework.marker {
        match marker.marker_type.as_str() {
            "attribute" => {
                if let Some(result) = try_match_attribute_marker(node, source, marker, framework) {
                    return Some(result);
                }
            }
            "name_pattern" => {
                if let Some(result) = try_match_name_pattern_marker(node, source, marker, framework) {
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
    let name = node_text(name_node, source);

    let normalized = normalize_name(&name, framework);

    match marker.creates.as_str() {
        "spec" => Some(SpecNode {
            name: normalized,
            kind: SpecKind::Spec,
            children: vec![],
            line: node.start_position().row + 1,
            parameterized: None,
        }),
        "group" => {
            let body = node.child_by_field_name("body")?;
            let children = parse_children(body, source, framework);
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
            let attr_text = node_text(sib, source);
            if let Some(arg) = attr_argument {
                if attr_text.contains(attr_name) && attr_text.contains(arg) {
                    return true;
                }
            } else if attr_text.contains(attr_name) {
                return true;
            }
        } else if sib.kind() != "attribute_item" && sib.kind() != "line_comment" && sib.kind() != "block_comment" {
            break;
        }
        sibling = sib.prev_sibling();
    }

    false
}

fn try_match_name_pattern_marker(
    node: Node,
    source: &str,
    marker: &crate::parse::registry::MarkerDef,
    framework: &FrameworkDef,
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
    let name = node_text(name_node, source);

    let pattern = marker.pattern.as_deref()?;
    if !matches_pattern(&name, pattern) {
        return None;
    }

    let normalized = normalize_name(&name, framework);

    match marker.creates.as_str() {
        "spec" => Some(SpecNode {
            name: normalized,
            kind: SpecKind::Spec,
            children: vec![],
            line: node.start_position().row + 1,
            parameterized: None,
        }),
        "group" => {
            let body_node = node.child_by_field_name("body")
                .or_else(|| {
                    let mut c = node.walk();
                    node.children(&mut c).find(|n| n.kind() == "body_statement" || n.kind() == "block")
                });
            let children = if let Some(body) = body_node {
                parse_children(body, source, framework)
            } else {
                vec![]
            };
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

    #[test]
    fn parse_rspec_fixture_base() {
        let source = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../specdiff-tests/fixtures/rspec/base/spec/models/user_spec.rb")
        );
        if let Ok(source) = source {
            let tree = parse_file(&source, "spec/models/user_spec.rb", rspec_framework());
            let tree = tree.expect("parsed fixture");
            let user = &tree.root[0];
            assert_eq!(user.children.len(), 2);
        }
    }

    #[test]
    fn parse_rust_fixture_base() {
        let source = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../specdiff-tests/fixtures/rust_builtin/base/src/lib.rs")
        );
        if let Ok(source) = source {
            let tree = parse_file(&source, "src/lib.rs", rust_framework());
            let tree = tree.expect("parsed fixture");
            assert_eq!(tree.root.len(), 1);
            let tests_mod = &tree.root[0];
            assert_eq!(tests_mod.children.len(), 3);
        }
    }

    #[test]
    fn normalize_rust_test_names() {
        let fw = rust_framework();
        assert_eq!(normalize_name("test_addition", fw), "addition");
        assert_eq!(normalize_name("test_my_func", fw), "my func");
        assert_eq!(normalize_name("helper_func", fw), "helper func");
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
        let source = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../specdiff-tests/fixtures/minitest/base/test/models/user_test.rb")
        );
        if let Ok(source) = source {
            let tree = parse_file(&source, "test/models/user_test.rb", minitest_framework());
            let tree = tree.expect("parsed fixture");
            assert_eq!(tree.root.len(), 1);
            let user_class = &tree.root[0];
            assert_eq!(user_class.children.len(), 3);
        }
    }
}
