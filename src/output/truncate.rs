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
}
