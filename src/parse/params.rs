use crate::parse::engine::node_text;
use crate::parse::registry::MarkerDef;
use tree_sitter::Node;

pub fn takes_required_param(function: Node, source: &str, marker: &MarkerDef) -> bool {
    let Some(required) = marker.required_param_type.as_deref() else {
        return true;
    };
    let Some(params) = function.child_by_field_name("parameters") else {
        return false;
    };
    let accepted = spellings(required, function, source);
    let mut cursor = params.walk();
    params.named_children(&mut cursor).any(|param| {
        param
            .child_by_field_name("type")
            .and_then(|t| node_text(t, source))
            .is_some_and(|t| accepted.contains(&t.split_whitespace().collect::<String>()))
    })
}

fn spellings(required: &str, function: Node, source: &str) -> Vec<String> {
    let stars = &required[..required.len() - required.trim_start_matches('*').len()];
    let Some((package, name)) = required[stars.len()..].rsplit_once('.') else {
        return vec![required.to_string()];
    };
    let Some(local_names) = import_names(function, source, package) else {
        return vec![required.to_string()];
    };
    local_names
        .into_iter()
        .filter(|local| local != "_")
        .map(|local| if local == "." { format!("{stars}{name}") } else { format!("{stars}{local}.{name}") })
        .collect()
}

fn import_names(node: Node, source: &str, package: &str) -> Option<Vec<String>> {
    let mut root = node;
    while let Some(parent) = root.parent() {
        root = parent;
    }
    let mut specs = Vec::new();
    let mut cursor = root.walk();
    let preamble = root
        .children(&mut cursor)
        .take_while(|n| matches!(n.kind(), "package_clause" | "import_declaration" | "comment"));
    for declaration in preamble.filter(|n| n.kind() == "import_declaration") {
        collect_import_specs(declaration, &mut specs);
    }
    let names: Vec<String> = specs
        .into_iter()
        .filter(|spec| {
            spec.child_by_field_name("path")
                .and_then(|p| node_text(p, source))
                .is_some_and(|p| p.trim_matches(|c| c == '"' || c == '`') == package)
        })
        .map(|spec| spec.child_by_field_name("name").and_then(|n| node_text(n, source)).unwrap_or_else(|| package.to_string()))
        .collect();
    (!names.is_empty()).then_some(names)
}

fn collect_import_specs<'a>(node: Node<'a>, specs: &mut Vec<Node<'a>>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "import_spec" {
            specs.push(child);
        } else {
            collect_import_specs(child, specs);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::parse::SpecNode;
    use crate::parse::engine::parse_file;
    use crate::parse::registry::all_frameworks;

    fn go(source: &str) -> Vec<SpecNode> {
        let go = all_frameworks().iter().find(|f| f.name == "go_testing").expect("go");
        parse_file(source, "user_test.go", go).map(|t| t.root).unwrap_or_default()
    }

    #[test]
    fn test_main_and_other_signatures_are_not_tests() {
        let source = "package user\n\nimport \"testing\"\n\nfunc TestMain(m *testing.M) {}\n\nfunc TestHelper() {}\n\nfunc TestReal(t *testing.T) {}\n";
        let names: Vec<String> = go(source).into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["Real"]);
    }

    #[test]
    fn a_renamed_or_dot_imported_testing_package_still_marks_a_test() {
        let aliased = "package user\n\nimport tt \"testing\"\n\nfunc TestAliased(t *tt.T) {}\n\nfunc TestMain(m *tt.M) {}\n";
        let names: Vec<String> = go(aliased).into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["Aliased"]);

        let dotted = "package user\n\nimport (\n\t\"fmt\"\n\t. \"testing\"\n)\n\nfunc TestDotted(t *T) {}\n\nfunc TestUnnamed(*T) {}\n\nfunc TestQualified(t *testing.T) {}\n";
        let names: Vec<String> = go(dotted).into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["Dotted", "Unnamed"]);
    }

    #[test]
    fn a_testing_package_imported_only_for_side_effects_marks_nothing() {
        let blank = "package user\n\nimport _ \"testing\"\n\nfunc TestThing(t *testing.T) {}\n";
        assert!(go(blank).is_empty());
    }

    proptest::proptest! {
        #[test]
        fn a_go_test_is_exactly_a_test_function_taking_a_testing_t(
            param in "[a-z]{1,4}",
            alias in proptest::option::of("[a-z]{1,4}"),
            qualifier in proptest::option::of("[a-z]{1,4}"),
            ty in proptest::sample::select(vec!["*T", "*M", "*B", "T", "int"]),
        ) {
            let keywords = ["go", "if", "for", "func", "var", "type", "map", "chan", "case", "else"];
            proptest::prop_assume!(
                [Some(param.as_str()), alias.as_deref(), qualifier.as_deref()].iter().flatten().all(|w| !keywords.contains(w))
            );
            let import = alias.as_deref().map_or_else(String::new, |a| format!("{a} "));
            let local = alias.as_deref().unwrap_or("testing");
            let qualifier = qualifier.as_deref().unwrap_or(local);
            let spelled = match ty.strip_prefix('*') {
                Some(name) => format!("*{qualifier}.{name}"),
                None if ty == "T" => format!("{qualifier}.T"),
                None => ty.to_string(),
            };
            let source = format!("package user\n\nimport {import}\"testing\"\n\nfunc TestThing({param} {spelled}) {{}}\n");
            let found = go(&source).len();
            proptest::prop_assert_eq!(found, usize::from(ty == "*T" && qualifier == local));
        }
    }
}
