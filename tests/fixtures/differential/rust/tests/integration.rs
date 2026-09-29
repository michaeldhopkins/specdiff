mod common;

#[test]
fn test_end_to_end() {
    assert_eq!(common::helper(), 1);
}

#[test]
#[ignore]
fn slow_end_to_end() {}
