pub fn path_names(path: &str, attr_name: &str, describable: bool) -> bool {
    path == attr_name || (describable && path.split_once("::").is_some_and(|(head, label)| head == attr_name && !label.contains("::")))
}

#[cfg(test)]
mod tests {
    use super::path_names;

    #[test]
    fn a_marker_is_its_exact_path() {
        assert!(path_names("test", "test", false));
        assert!(!path_names("test::custom", "test", false));
        assert!(!path_names("rstest::fixture", "rstest", false));
        assert!(!path_names("tokio::test", "test", false));
    }

    #[test]
    fn a_describable_attribute_may_carry_one_label() {
        assert!(path_names("case", "case", true));
        assert!(path_names("case::empty_input", "case", true));
        assert!(!path_names("case::a::b", "case", true));
        assert!(!path_names("cases::x", "case", true));
        assert!(!path_names("rstest::case", "case", true));
    }

    proptest::proptest! {
        #[test]
        fn only_the_exact_name_or_one_label_under_it_names_an_attribute(
            head in "[a-z]{1,6}",
            label in proptest::option::of("[a-z_]{1,8}"),
            describable in proptest::bool::ANY,
        ) {
            let path = label.as_deref().map_or_else(|| head.clone(), |l| format!("{head}::{l}"));
            proptest::prop_assert!(path_names(&path, &head, describable) == (label.is_none() || describable));
            let longer = format!("{head}x");
            proptest::prop_assert!(!path_names(&path, &longer, describable));
        }
    }
}
