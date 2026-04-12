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
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
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
            node_text(method_node, source)
        }
        "call_expression" => {
            let function_node = node.child_by_field_name("function")?;
            node_text(function_node, source)
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

fn find_block(node: Node) -> Option<Node> {
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
                    "string_content" | "string_fragment" | "interpreted_string_literal_content" => {
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
    let name = node_text(name_node, source)?;

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
                Some(SpecNode {
                    name: normalized,
                    kind: SpecKind::Spec,
                    children: vec![],
                    line: node.start_position().row + 1,
                    parameterized: None,
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
}
