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
    let mut cursor = params.walk();
    params.named_children(&mut cursor).any(|param| {
        param
            .child_by_field_name("type")
            .and_then(|t| node_text(t, source))
            .is_some_and(|t| t == required)
    })
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

    proptest::proptest! {
        #[test]
        fn a_go_test_is_exactly_a_test_function_taking_a_testing_t(
            param in "[a-z]{1,4}",
            ty in proptest::sample::select(vec!["*testing.T", "*testing.M", "*testing.B", "testing.T", "int"]),
        ) {
            let source = format!("package user\n\nimport \"testing\"\n\nfunc TestThing({param} {ty}) {{}}\n");
            let found = go(&source).len();
            proptest::prop_assert_eq!(found, usize::from(ty == "*testing.T"));
        }
    }
}
