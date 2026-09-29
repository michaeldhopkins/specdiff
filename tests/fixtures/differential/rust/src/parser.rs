pub fn parse(s: &str) -> usize {
    s.len()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parses_empty() {
        assert_eq!(super::parse(""), 0);
    }
}

#[cfg(test)]
mod edge_cases {
    #[test]
    fn long_input() {
        assert_eq!(super::parse("abc"), 3);
    }
}
