use crate::parse::registry::FrameworkDef;
use crate::parse::SpecNode;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct SharedExampleRegistry {
    definitions: HashMap<String, Vec<SpecNode>>,
    types: HashMap<String, Vec<SpecNode>>,
}

impl SharedExampleRegistry {
    pub fn register(&mut self, name: String, specs: Vec<SpecNode>) {
        self.definitions.insert(name, specs);
    }

    pub fn register_type(&mut self, name: String, specs: Vec<SpecNode>) {
        self.types.insert(name, specs);
    }

    pub fn get(&self, name: &str) -> Option<&[SpecNode]> {
        self.definitions.get(name).map(|v| v.as_slice())
    }

    pub fn get_type(&self, name: &str) -> Option<&[SpecNode]> {
        self.types.get(name).map(|v| v.as_slice())
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty() && self.types.is_empty()
    }

    pub fn len(&self) -> usize {
        self.definitions.len()
    }
}

pub fn scan_for_definitions(
    source: &str,
    framework: &FrameworkDef,
    registry: &mut SharedExampleRegistry,
) {
    let language = match crate::parse::engine::language_for_framework(framework) {
        Some(l) => l,
        None => return,
    };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return;
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return,
    };

    if let Some(shared) = &framework.shared
        && !shared.definition.is_empty()
    {
        scan_node(tree.root_node(), source, framework, shared, registry);
    }

    scan_types(tree.root_node(), source, framework, registry);
}

fn scan_types(
    node: tree_sitter::Node,
    source: &str,
    framework: &FrameworkDef,
    registry: &mut SharedExampleRegistry,
) {
    let type_node_kinds: &[&str] = match framework.language.as_str() {
        "python" => &["class_definition"],
        "ruby" => &["class", "module"],
        _ => return,
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if type_node_kinds.contains(&child.kind())
            && let Some((name, specs)) = extract_type_definition(child, source, framework)
        {
            registry.register_type(name, specs);
        }
        scan_types(child, source, framework, registry);
    }
}

fn extract_type_definition(
    node: tree_sitter::Node,
    source: &str,
    framework: &FrameworkDef,
) -> Option<(String, Vec<SpecNode>)> {
    let name_node = match framework.language.as_str() {
        "python" => node.child_by_field_name("name")?,
        "ruby" => {
            let mut cursor = node.walk();
            node.children(&mut cursor).find(|c| c.kind() == "constant")?
        }
        _ => return None,
    };
    let name = crate::parse::engine::node_text_pub(name_node, source)?;

    let body = match framework.language.as_str() {
        "python" => node.child_by_field_name("body")?,
        "ruby" => {
            let mut cursor = node.walk();
            node.children(&mut cursor).find(|c| c.kind() == "body_statement")?
        }
        _ => return None,
    };

    let specs = crate::parse::engine::parse_children(body, source, framework);
    if specs.is_empty() {
        return None;
    }
    Some((name, specs))
}

fn scan_node(
    node: tree_sitter::Node,
    source: &str,
    framework: &FrameworkDef,
    shared: &crate::parse::registry::SharedDef,
    registry: &mut SharedExampleRegistry,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if (child.kind() == "call" || child.kind() == "call_expression")
            && let Some((name, specs)) = try_match_definition(child, source, framework, shared)
        {
            registry.register(name, specs);
            continue;
        }
        scan_node(child, source, framework, shared, registry);
    }
}

fn try_match_definition(
    node: tree_sitter::Node,
    source: &str,
    framework: &FrameworkDef,
    shared: &crate::parse::registry::SharedDef,
) -> Option<(String, Vec<SpecNode>)> {
    let method_name = crate::parse::engine::extract_method_name(node, source)?;

    for def in &shared.definition {
        if def.ast_type != node.kind() {
            continue;
        }
        if !def.method_names.contains(&method_name) {
            continue;
        }

        let name = crate::parse::engine::extract_name(
            node,
            source,
            &def.name_source,
            def.name_source_type.as_deref(),
        )?;

        let block = crate::parse::engine::find_block(node)?;
        let children = crate::parse::engine::parse_children(block, source, framework);

        return Some((name, children));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::registry::all_frameworks;

    fn rspec() -> &'static FrameworkDef {
        all_frameworks().iter().find(|f| f.name == "rspec").expect("rspec")
    }

    #[test]
    fn scan_rspec_shared_examples() {
        let source = r#"
RSpec.shared_examples "a timestamped model" do
  it "has created_at" do
    expect(subject).to respond_to(:created_at)
  end

  it "has updated_at" do
    expect(subject).to respond_to(:updated_at)
  end
end
"#;
        let mut registry = SharedExampleRegistry::default();
        scan_for_definitions(source, rspec(), &mut registry);

        assert_eq!(registry.len(), 1);
        let specs = registry.get("a timestamped model").expect("found");
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "has created_at");
        assert_eq!(specs[1].name, "has updated_at");
    }

    #[test]
    fn scan_multiple_shared_examples() {
        let source = r#"
RSpec.shared_examples "validatable" do
  it "validates presence" do
  end
end

RSpec.shared_context "authenticated" do
  it "sets current user" do
  end
end
"#;
        let mut registry = SharedExampleRegistry::default();
        scan_for_definitions(source, rspec(), &mut registry);

        assert_eq!(registry.len(), 2);
        assert!(registry.get("validatable").is_some());
        assert!(registry.get("authenticated").is_some());
    }

    #[test]
    fn scan_empty_source_finds_nothing() {
        let mut registry = SharedExampleRegistry::default();
        scan_for_definitions("", rspec(), &mut registry);
        assert!(registry.is_empty());
    }

    #[test]
    fn scan_no_shared_section_is_noop() {
        let fw = all_frameworks().iter().find(|f| f.name == "rust_builtin").expect("rust");
        let mut registry = SharedExampleRegistry::default();
        scan_for_definitions("#[test]\nfn test_foo() {}", fw, &mut registry);
        assert!(registry.is_empty());
    }

    #[test]
    fn resolve_include_examples_inlines_specs() {
        let shared_source = r#"
RSpec.shared_examples "a timestamped model" do
  it "has created_at" do
  end
  it "has updated_at" do
  end
end
"#;
        let mut registry = SharedExampleRegistry::default();
        scan_for_definitions(shared_source, rspec(), &mut registry);

        let test_source = r#"
RSpec.describe User do
  include_examples "a timestamped model"

  it "has a name" do
  end
end
"#;
        let tree = crate::parse::engine::parse_file_with_shared(
            test_source, "spec/models/user_spec.rb", rspec(), Some(&registry),
        );
        let tree = tree.expect("parsed with shared");
        let user = &tree.root[0];
        assert_eq!(user.children.len(), 3, "should have 2 inlined + 1 own spec");
        assert_eq!(user.children[0].name, "has created_at");
        assert_eq!(user.children[1].name, "has updated_at");
        assert_eq!(user.children[2].name, "has a name");
    }

    #[test]
    fn resolve_it_behaves_like_nests_specs() {
        let shared_source = r#"
RSpec.shared_examples "a timestamped model" do
  it "has created_at" do
  end
  it "has updated_at" do
  end
end
"#;
        let mut registry = SharedExampleRegistry::default();
        scan_for_definitions(shared_source, rspec(), &mut registry);

        let test_source = r#"
RSpec.describe User do
  it_behaves_like "a timestamped model"

  it "has a name" do
  end
end
"#;
        let tree = crate::parse::engine::parse_file_with_shared(
            test_source, "spec/models/user_spec.rb", rspec(), Some(&registry),
        );
        let tree = tree.expect("parsed with shared");
        let user = &tree.root[0];
        assert_eq!(user.children.len(), 2, "should have 1 nested group + 1 own spec");

        let behaves_like = &user.children[0];
        assert_eq!(behaves_like.name, "behaves like a timestamped model");
        assert_eq!(behaves_like.kind, crate::parse::SpecKind::Group);
        assert_eq!(behaves_like.children.len(), 2);
        assert_eq!(behaves_like.children[0].name, "has created_at");

        assert_eq!(user.children[1].name, "has a name");
    }

    #[test]
    fn unresolved_inclusion_is_ignored() {
        let registry = SharedExampleRegistry::default();

        let test_source = r#"
RSpec.describe User do
  include_examples "nonexistent"
  it "has a name" do
  end
end
"#;
        let tree = crate::parse::engine::parse_file_with_shared(
            test_source, "spec/models/user_spec.rb", rspec(), Some(&registry),
        );
        let tree = tree.expect("parsed");
        let user = &tree.root[0];
        assert_eq!(user.children.len(), 1, "unresolved inclusion should be skipped");
        assert_eq!(user.children[0].name, "has a name");
    }

    #[test]
    fn without_registry_inclusions_are_ignored() {
        let test_source = r#"
RSpec.describe User do
  include_examples "a timestamped model"
  it "has a name" do
  end
end
"#;
        let tree = crate::parse::engine::parse_file(
            test_source, "spec/models/user_spec.rb", rspec(),
        );
        let tree = tree.expect("parsed");
        let user = &tree.root[0];
        assert_eq!(user.children.len(), 1, "without registry, inclusions are ignored");
    }
}
