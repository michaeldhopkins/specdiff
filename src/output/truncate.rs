pub const CONTEXT_HEAD: usize = 3;
pub const CONTEXT_TAIL: usize = 2;

pub fn truncate_unchanged_runs<T, F, G>(
    lines: &mut Vec<T>,
    head: usize,
    tail: usize,
    is_unchanged: F,
    make_ellipsis: G,
) where
    F: Fn(&T) -> bool,
    G: Fn(usize) -> T,
{
    let mut i = 0;
    while i < lines.len() {
        if !is_unchanged(&lines[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && is_unchanged(&lines[i]) {
            i += 1;
        }
        let run_len = i - start;
        if run_len > head + tail + 1 {
            let hidden = run_len - head - tail;
            lines.splice(
                start + head..start + head + hidden,
                std::iter::once(make_ellipsis(hidden)),
            );
            i = start + head + 1 + tail;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Tok {
        U(u32),
        C(u32),
        Ellipsis(usize),
    }

    fn truncate(lines: &mut Vec<Tok>, head: usize, tail: usize) {
        truncate_unchanged_runs(
            lines,
            head,
            tail,
            |t| matches!(t, Tok::U(_)),
            Tok::Ellipsis,
        );
    }

    #[test]
    fn long_run_truncates_to_head_ellipsis_tail() {
        let mut lines: Vec<Tok> = (0..10).map(Tok::U).chain(std::iter::once(Tok::C(0))).collect();
        truncate(&mut lines, 3, 2);
        assert_eq!(
            lines,
            vec![
                Tok::U(0),
                Tok::U(1),
                Tok::U(2),
                Tok::Ellipsis(5),
                Tok::U(8),
                Tok::U(9),
                Tok::C(0),
            ]
        );
    }

    #[test]
    fn run_at_threshold_stays_intact() {
        let mut lines: Vec<Tok> = (0..6).map(Tok::U).collect();
        let original = lines.clone();
        truncate(&mut lines, 3, 2);
        assert_eq!(lines, original, "run of head+tail+1 should not truncate");
    }

    #[test]
    fn run_just_over_threshold_truncates() {
        let mut lines: Vec<Tok> = (0..7).map(Tok::U).collect();
        truncate(&mut lines, 3, 2);
        assert_eq!(
            lines,
            vec![
                Tok::U(0),
                Tok::U(1),
                Tok::U(2),
                Tok::Ellipsis(2),
                Tok::U(5),
                Tok::U(6),
            ]
        );
    }

    #[test]
    fn changed_lines_break_runs() {
        let mut lines = vec![
            Tok::U(0), Tok::U(1), Tok::U(2), Tok::U(3), Tok::U(4),
            Tok::U(5), Tok::U(6), Tok::U(7),
            Tok::C(99),
            Tok::U(10), Tok::U(11), Tok::U(12), Tok::U(13),
            Tok::U(14), Tok::U(15), Tok::U(16),
        ];
        truncate(&mut lines, 3, 2);
        assert_eq!(
            lines,
            vec![
                Tok::U(0), Tok::U(1), Tok::U(2),
                Tok::Ellipsis(3),
                Tok::U(6), Tok::U(7),
                Tok::C(99),
                Tok::U(10), Tok::U(11), Tok::U(12),
                Tok::Ellipsis(2),
                Tok::U(15), Tok::U(16),
            ]
        );
    }

    #[test]
    fn zero_context_collapses_a_leading_run_to_the_ellipsis_alone() {
        let mut lines = vec![Tok::U(0), Tok::U(1), Tok::C(0), Tok::U(2)];
        truncate(&mut lines, 0, 0);
        assert_eq!(lines, vec![Tok::Ellipsis(2), Tok::C(0), Tok::U(2)]);
    }

    #[test]
    fn tail_longer_than_head_keeps_a_leading_run_intact_at_both_ends() {
        let mut lines: Vec<Tok> = (0..8).map(Tok::U).chain(std::iter::once(Tok::C(0))).collect();
        truncate(&mut lines, 1, 3);
        assert_eq!(
            lines,
            vec![
                Tok::U(0),
                Tok::Ellipsis(4),
                Tok::U(5),
                Tok::U(6),
                Tok::U(7),
                Tok::C(0),
            ]
        );
    }

    #[test]
    fn empty_vec_is_a_noop() {
        let mut lines: Vec<Tok> = vec![];
        truncate(&mut lines, 3, 2);
        assert!(lines.is_empty());
    }

    #[test]
    fn only_changed_is_a_noop() {
        let mut lines = vec![Tok::C(0), Tok::C(1), Tok::C(2)];
        let original = lines.clone();
        truncate(&mut lines, 3, 2);
        assert_eq!(lines, original);
    }

    fn reference_truncate(lines: &[Tok], head: usize, tail: usize) -> Vec<Tok> {
        let mut out: Vec<Tok> = Vec::with_capacity(lines.len());
        let mut i = 0;
        while i < lines.len() {
            if !matches!(lines[i], Tok::U(_)) {
                out.push(lines[i].clone());
                i += 1;
                continue;
            }
            let start = i;
            while i < lines.len() && matches!(lines[i], Tok::U(_)) {
                i += 1;
            }
            let run_len = i - start;
            if run_len > head + tail + 1 {
                let hidden = run_len - head - tail;
                for j in 0..head {
                    out.push(lines[start + j].clone());
                }
                out.push(Tok::Ellipsis(hidden));
                for j in 0..tail {
                    out.push(lines[start + head + hidden + j].clone());
                }
            } else {
                for j in 0..run_len {
                    out.push(lines[start + j].clone());
                }
            }
        }
        out
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn truncate_unchanged_runs_matches_a_simple_reference_implementation(
            input in proptest::collection::vec(
                (0u32..16, proptest::bool::ANY).prop_map(|(n, u)| if u { Tok::U(n) } else { Tok::C(n) }),
                0..40,
            ),
            head in 0usize..4,
            tail in 0usize..4,
        ) {
            let mut actual = input.clone();
            truncate(&mut actual, head, tail);
            let expected = reference_truncate(&input, head, tail);
            prop_assert_eq!(actual, expected);
        }

        #[test]
        fn truncate_unchanged_runs_is_idempotent(
            input in proptest::collection::vec(
                (0u32..16, proptest::bool::ANY).prop_map(|(n, u)| if u { Tok::U(n) } else { Tok::C(n) }),
                0..40,
            ),
            head in 0usize..4,
            tail in 0usize..4,
        ) {
            let mut once = input.clone();
            truncate(&mut once, head, tail);
            let mut twice = once.clone();
            truncate(&mut twice, head, tail);
            prop_assert_eq!(once, twice);
        }

        #[test]
        fn truncate_unchanged_runs_keeps_every_maximal_unchanged_run_within_the_head_plus_tail_plus_one_window(
            input in proptest::collection::vec(
                (0u32..16, proptest::bool::ANY).prop_map(|(n, u)| if u { Tok::U(n) } else { Tok::C(n) }),
                0..40,
            ),
            head in 0usize..4,
            tail in 0usize..4,
        ) {
            let mut lines = input.clone();
            truncate(&mut lines, head, tail);
            let mut i = 0;
            while i < lines.len() {
                if !matches!(lines[i], Tok::U(_)) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < lines.len() && matches!(lines[i], Tok::U(_)) {
                    i += 1;
                }
                let run_len = i - start;
                prop_assert!(
                    run_len <= head + tail + 1,
                    "maximal run of {} unchanged items at index {} exceeds head+tail+1={}",
                    run_len,
                    start,
                    head + tail + 1,
                );
            }
        }

        #[test]
        fn truncate_unchanged_runs_conserves_the_total_number_of_unchanged_items_via_ellipsis_counts(
            input in proptest::collection::vec(
                (0u32..16, proptest::bool::ANY).prop_map(|(n, u)| if u { Tok::U(n) } else { Tok::C(n) }),
                0..40,
            ),
            head in 0usize..4,
            tail in 0usize..4,
        ) {
            let unchanged_in_input = input.iter().filter(|t| matches!(t, Tok::U(_))).count();
            let mut lines = input.clone();
            truncate(&mut lines, head, tail);
            let unchanged_in_output = lines
                .iter()
                .map(|t| match t {
                    Tok::U(_) => 1,
                    Tok::Ellipsis(n) => *n,
                    Tok::C(_) => 0,
                })
                .sum::<usize>();
            prop_assert_eq!(unchanged_in_input, unchanged_in_output);
        }
    }
}
