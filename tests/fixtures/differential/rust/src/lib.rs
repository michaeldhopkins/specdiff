pub mod deep;
pub mod parser;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_addition() {
        assert_eq!(add(2, 2), 4);
    }

    #[test]
    fn plain_name_without_prefix() {
        assert_eq!(add(1, 1), 2);
    }

    #[test]
    #[ignore = "slow"]
    fn ignored_test() {}

    #[test]
    #[should_panic(expected = "boom")]
    fn panics_on_purpose() {
        panic!("boom");
    }

    #[test]
    #[cfg(feature = "extra")]
    fn only_with_the_extra_feature() {}

    #[cfg(not(feature = "extra"))]
    #[test]
    fn only_without_the_extra_feature() {}

    #[rstest]
    #[case(1, 1, 2)]
    #[case(2, 3, 5)]
    #[case(10, 20, 30)]
    fn test_add_cases(#[case] a: i32, #[case] b: i32, #[case] expected: i32) {
        assert_eq!(add(a, b), expected);
    }

    #[rstest]
    #[case::zero(0, 0)]
    #[case::one(1, 2)]
    fn described_cases(#[case] a: i32, #[case] doubled: i32) {
        assert_eq!(add(a, a), doubled);
    }

    #[rstest]
    fn matrix(#[values(1, 2)] a: i32, #[values(3, 4)] b: i32) {
        assert!(add(a, b) > 0);
    }

    #[rstest]
    fn rstest_without_cases() {}

    proptest::proptest! {
        #[test]
        fn addition_commutes(a in 0i32..100, b in 0i32..100) {
            proptest::prop_assert_eq!(add(a, b), add(b, a));
        }
    }

    macro_rules! generated {
        ($($name:ident),*) => { $( #[test] fn $name() {} )* };
    }
    generated!(first_generated, second_generated);

    mod inner {
        #[test]
        fn nested_module_test() {}
    }
}
