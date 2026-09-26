pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adds() {
        assert_eq!(add(1, 2), 3);
    }

    #[rstest]
    #[case(1, 2)]
    #[case(3, 4)]
    fn test_cases(#[case] a: i32, #[case] b: i32) {
    }

    proptest! {
        #[test]
        fn add_commutes(a in 0..10i32, b in 0..10i32) {
            prop_assert_eq!(add(a, b), add(b, a));
        }
    }

    mod nested {
        #[test]
        #[ignore]
        fn slow_one() {}
    }
}
