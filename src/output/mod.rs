pub mod truncate;

use crate::diff::types::{DiffKind, DiffNode, FileDiff, Stats};
use std::fmt::Write;
use std::io::IsTerminal;
use truncate::{truncate_unchanged_runs, CONTEXT_HEAD, CONTEXT_TAIL};

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

#[derive(Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct TreeOptions {
    pub changed_only: bool,
    pub color: bool,
    pub full_context: bool,
}

#[derive(Clone, Copy)]
enum LineKind {
    Spec(DiffKind),
    Other,
    Ellipsis(usize),
}

struct RenderedLine {
    kind: LineKind,
    text: String,
}

pub fn format_json(file_diffs: &[FileDiff]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(file_diffs)?)
}

pub fn format_tree(file_diffs: &[FileDiff], opts: TreeOptions) -> String {
    let use_color = opts.color
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none();

    let mut lines: Vec<RenderedLine> = Vec::new();
    push_stats_header(&mut lines, file_diffs, use_color);

    for file_diff in file_diffs {
        let file_has_changes = file_diff.nodes.iter().any(DiffNode::has_changes);
        if opts.changed_only && !file_has_changes {
            continue;
        }
        push_file_header(&mut lines, &file_diff.path, use_color);
        for node in &file_diff.nodes {
            push_tree_node(node, &mut lines, 1, opts.changed_only, use_color);
        }
    }

    if !opts.changed_only && !opts.full_context {
        truncate_unchanged_runs(
            &mut lines,
            CONTEXT_HEAD,
            CONTEXT_TAIL,
            |line| matches!(line.kind, LineKind::Spec(DiffKind::Unchanged)),
            |hidden| make_ellipsis_line(hidden, use_color),
        );
    }

    let mut output = String::new();
    for line in lines {
        let _ = writeln!(output, "{}", line.text);
    }
    output
}

pub fn format_compact(file_diffs: &[FileDiff]) -> String {
    let mut output = String::new();
    for file_diff in file_diffs {
        collect_compact_lines(&file_diff.nodes, &[file_diff.path.as_str()], &mut output);
    }
    output
}

fn push_stats_header(lines: &mut Vec<RenderedLine>, file_diffs: &[FileDiff], use_color: bool) {
    let stats = Stats::from_file_diffs(file_diffs);
    if stats.is_empty() {
        return;
    }
    let mut text = String::new();
    if use_color {
        let _ = write!(text, "{BOLD}");
    }
    let _ = write!(text, "specdiff");
    if use_color {
        let _ = write!(text, "{RESET}");
    }
    let _ = write!(text, "  ");
    if stats.added > 0 {
        push_stat(&mut text, GREEN, '+', stats.added, use_color);
    }
    if stats.removed > 0 {
        push_stat(&mut text, RED, '-', stats.removed, use_color);
    }
    if stats.renamed > 0 {
        push_stat_str(&mut text, YELLOW, "~>", stats.renamed, use_color);
    }
    if stats.modified > 0 {
        push_stat(&mut text, CYAN, '~', stats.modified, use_color);
    }
    lines.push(RenderedLine { kind: LineKind::Other, text });
    lines.push(RenderedLine { kind: LineKind::Other, text: String::new() });
}

fn push_stat(text: &mut String, color: &str, glyph: char, count: usize, use_color: bool) {
    if use_color {
        let _ = write!(text, "{color}{glyph}{count}{RESET} ");
    } else {
        let _ = write!(text, "{glyph}{count} ");
    }
}

fn push_stat_str(text: &mut String, color: &str, glyph: &str, count: usize, use_color: bool) {
    if use_color {
        let _ = write!(text, "{color}{glyph}{count}{RESET} ");
    } else {
        let _ = write!(text, "{glyph}{count} ");
    }
}

fn push_file_header(lines: &mut Vec<RenderedLine>, path: &str, use_color: bool) {
    let text = if use_color {
        format!("{BOLD}  {path}{RESET}")
    } else {
        format!("  {path}")
    };
    lines.push(RenderedLine { kind: LineKind::Other, text });
}

fn make_ellipsis_line(hidden: usize, use_color: bool) -> RenderedLine {
    let plural = if hidden == 1 { "" } else { "s" };
    let body = format!("    ... ({hidden} hidden line{plural})");
    let text = if use_color {
        format!("{DIM}{body}{RESET}")
    } else {
        body
    };
    RenderedLine { kind: LineKind::Ellipsis(hidden), text }
}

fn push_tree_node(
    node: &DiffNode,
    lines: &mut Vec<RenderedLine>,
    depth: usize,
    changed_only: bool,
    color: bool,
) {
    if changed_only && node.kind == DiffKind::Unchanged && !node.has_changes() {
        return;
    }

    let indent = "  ".repeat(depth);
    let (prefix, color_start, color_end) = if color {
        match node.kind {
            DiffKind::Added => ("+ ", GREEN, RESET),
            DiffKind::Removed => ("- ", RED, RESET),
            DiffKind::Renamed => ("->", YELLOW, RESET),
            DiffKind::Modified => ("~ ", CYAN, RESET),
            DiffKind::Unchanged => ("  ", DIM, RESET),
        }
    } else {
        match node.kind {
            DiffKind::Added => ("+ ", "", ""),
            DiffKind::Removed => ("- ", "", ""),
            DiffKind::Renamed => ("->", "", ""),
            DiffKind::Modified => ("~ ", "", ""),
            DiffKind::Unchanged => ("  ", "", ""),
        }
    };

    let param_suffix = match (node.old_param_cases, node.param_cases) {
        (Some(old), Some(new)) => format!(" [{new} cases, was {old}]"),
        (Some(old), None) => format!(" [was {old} cases]"),
        (None, Some(n)) => format!(" [{n} cases]"),
        (None, None) => String::new(),
    };

    let text = match node.kind {
        DiffKind::Renamed => {
            if let Some(old) = &node.old_name {
                format!("{color_start}{prefix} {indent}{} (was {old}){param_suffix}{color_end}", node.name)
            } else {
                format!("{color_start}{prefix} {indent}{}{param_suffix}{color_end}", node.name)
            }
        }
        _ => format!("{color_start}{prefix} {indent}{}{param_suffix}{color_end}", node.name),
    };

    lines.push(RenderedLine { kind: LineKind::Spec(node.kind), text });

    for child in &node.children {
        push_tree_node(child, lines, depth + 1, changed_only, color);
    }
}

fn collect_compact_lines(nodes: &[DiffNode], path: &[&str], output: &mut String) {
    for node in nodes {
        if node.kind == DiffKind::Unchanged && !node.has_changes() {
            continue;
        }

        let mut current_path = path.to_vec();
        current_path.push(&node.name);
        let full_path = current_path.join(" > ");

        match node.kind {
            DiffKind::Added => {
                let _ = writeln!(output, "+ {full_path}");
            }
            DiffKind::Removed => {
                let _ = writeln!(output, "- {full_path}");
            }
            DiffKind::Renamed => {
                let old = node.old_name.as_deref().unwrap_or("?");
                let _ = writeln!(output, "-> {full_path} (was {old})");
            }
            DiffKind::Modified | DiffKind::Unchanged => {}
        }

        collect_compact_lines(&node.children, &current_path, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_plain(changed_only: bool) -> TreeOptions {
        TreeOptions { changed_only, color: false, full_context: true }
    }

    fn opts_truncating() -> TreeOptions {
        TreeOptions { changed_only: false, color: false, full_context: false }
    }

    fn unchanged(name: &str) -> DiffNode {
        DiffNode {
            name: name.into(),
            kind: DiffKind::Unchanged,
            old_name: None,
            param_cases: None,
            old_param_cases: None,
            children: vec![],
        }
    }

    fn added(name: &str) -> DiffNode {
        DiffNode {
            name: name.into(),
            kind: DiffKind::Added,
            old_name: None,
            param_cases: None,
            old_param_cases: None,
            children: vec![],
        }
    }

    fn sample_file_diffs() -> Vec<FileDiff> {
        vec![FileDiff {
            path: "models::user".into(),
            nodes: vec![
                DiffNode {
                    name: "validations".into(),
                    kind: DiffKind::Modified,
                    old_name: None,
                    param_cases: None,
                    old_param_cases: None,
                    children: vec![
                        unchanged("validates email"),
                        added("validates uniqueness"),
                    ],
                },
                DiffNode {
                    name: "associations".into(),
                    kind: DiffKind::Unchanged,
                    old_name: None,
                    param_cases: None,
                    old_param_cases: None,
                    children: vec![unchanged("has many posts")],
                },
            ],
        }]
    }

    #[test]
    fn tree_format_shows_file_path() {
        let output = format_tree(&sample_file_diffs(), opts_plain(false));
        assert!(output.contains("models::user"), "should show file path header");
        assert!(output.contains("validations"), "should show group");
        assert!(output.contains("validates uniqueness"), "should show added spec");
    }

    #[test]
    fn tree_format_shows_stats_header() {
        let output = format_tree(&sample_file_diffs(), opts_plain(false));
        assert!(output.contains("specdiff"), "should show header");
        assert!(output.contains("+1"), "should show added count (leaf specs only)");
    }

    #[test]
    fn tree_format_changed_only() {
        let output = format_tree(&sample_file_diffs(), opts_plain(true));
        assert!(output.contains("models::user"));
        assert!(output.contains("validations"));
        assert!(output.contains("validates uniqueness"));
        assert!(!output.contains("associations"));
    }

    #[test]
    fn compact_format_includes_file_path() {
        let output = format_compact(&sample_file_diffs());
        assert!(output.contains("models::user > validations > validates uniqueness"));
    }

    #[test]
    fn json_format_includes_file_path() {
        let diffs = sample_file_diffs();
        let json = format_json(&diffs).expect("json");
        assert!(json.contains("models::user"));
        assert!(json.contains("validates uniqueness"));
    }

    #[test]
    fn tree_format_no_color_has_no_escape_codes() {
        let output = format_tree(&sample_file_diffs(), opts_plain(false));
        assert!(!output.contains("\x1b["), "no-color output should not contain ANSI codes");
    }

    #[test]
    fn tree_format_empty_diffs_no_stats() {
        let output = format_tree(&[], opts_plain(false));
        assert!(!output.contains("+0"), "empty diffs should not show +0");
        assert!(!output.contains("-0"), "empty diffs should not show -0");
    }

    #[test]
    fn renamed_spec_with_param_delta_never_has_two_arrows_on_one_line() {
        let diffs = vec![FileDiff {
            path: "m::u".into(),
            nodes: vec![DiffNode {
                name: "validates shape".into(),
                kind: DiffKind::Renamed,
                old_name: Some("validates".into()),
                param_cases: Some(5),
                old_param_cases: Some(3),
                children: vec![],
            }],
        }];
        let output = format_tree(&diffs, opts_plain(false));
        let rename_line = output
            .lines()
            .find(|l| l.contains("validates shape"))
            .expect("rename line");
        assert_eq!(
            rename_line.matches("->").count(),
            1,
            "rename line must contain exactly one `->` (from the rename prefix), got: {rename_line}"
        );
        assert!(rename_line.contains("[5 cases, was 3]"));
    }

    #[test]
    fn param_suffix_prose_was_only() {
        let diffs = vec![FileDiff {
            path: "m::u".into(),
            nodes: vec![DiffNode {
                name: "lost parameters".into(),
                kind: DiffKind::Modified,
                old_name: None,
                param_cases: None,
                old_param_cases: Some(4),
                children: vec![],
            }],
        }];
        let output = format_tree(&diffs, opts_plain(false));
        assert!(output.contains("[was 4 cases]"), "unparameterized spec should show prior count");
    }

    #[test]
    fn tree_format_no_changes_shows_message() {
        let diffs = vec![FileDiff {
            path: "models::user".into(),
            nodes: vec![DiffNode {
                name: "works".into(),
                kind: DiffKind::Unchanged,
                old_name: None,
                param_cases: None,
                old_param_cases: None,
                children: vec![],
            }],
        }];
        let output = format_tree(&diffs, opts_plain(false));
        assert!(!output.contains("+0"), "all-unchanged should not show +0");
    }

    fn diffs_with_long_unchanged_run() -> Vec<FileDiff> {
        let mut children: Vec<DiffNode> = (0..10).map(|i| unchanged(&format!("ctx {i}"))).collect();
        children.push(added("new spec"));
        vec![FileDiff {
            path: "m::u".into(),
            nodes: vec![DiffNode {
                name: "group".into(),
                kind: DiffKind::Modified,
                old_name: None,
                param_cases: None,
                old_param_cases: None,
                children,
            }],
        }]
    }

    #[test]
    fn truncation_default_collapses_long_unchanged_run() {
        let output = format_tree(&diffs_with_long_unchanged_run(), opts_truncating());
        assert!(output.contains("ctx 0"), "first head line kept");
        assert!(output.contains("ctx 1"));
        assert!(output.contains("ctx 2"));
        assert!(!output.contains("ctx 3"), "middle of run dropped");
        assert!(!output.contains("ctx 4"));
        assert!(!output.contains("ctx 5"));
        assert!(!output.contains("ctx 6"));
        assert!(!output.contains("ctx 7"));
        assert!(output.contains("ctx 8"), "tail line kept");
        assert!(output.contains("ctx 9"));
        assert!(output.contains("... (5 hidden lines)"), "ellipsis line with count");
        assert!(output.contains("new spec"), "changed line still present");
        let new_spec_line = output
            .lines()
            .find(|l| l.contains("new spec"))
            .expect("new spec line");
        assert!(new_spec_line.trim_start().starts_with('+'), "new spec rendered as Added");
    }

    #[test]
    fn truncation_full_context_flag_disables_truncation() {
        let opts = TreeOptions { changed_only: false, color: false, full_context: true };
        let output = format_tree(&diffs_with_long_unchanged_run(), opts);
        for i in 0..10 {
            assert!(output.contains(&format!("ctx {i}")), "--full-context shows ctx {i}");
        }
        assert!(!output.contains("hidden line"), "no ellipsis with --full-context");
    }

    #[test]
    fn truncation_changed_only_drops_all_unchanged() {
        let opts = TreeOptions { changed_only: true, color: false, full_context: false };
        let output = format_tree(&diffs_with_long_unchanged_run(), opts);
        for i in 0..10 {
            assert!(!output.contains(&format!("ctx {i}")), "--changed-only drops ctx {i}");
        }
        assert!(!output.contains("hidden line"), "no ellipsis under --changed-only");
        assert!(output.contains("new spec"));
    }

    #[test]
    fn truncation_short_run_stays_intact() {
        let mut children: Vec<DiffNode> = (0..5).map(|i| unchanged(&format!("ctx {i}"))).collect();
        children.push(added("new"));
        let diffs = vec![FileDiff {
            path: "m::u".into(),
            nodes: vec![DiffNode {
                name: "group".into(),
                kind: DiffKind::Modified,
                old_name: None,
                param_cases: None,
                old_param_cases: None,
                children,
            }],
        }];
        let output = format_tree(&diffs, opts_truncating());
        for i in 0..5 {
            assert!(output.contains(&format!("ctx {i}")), "short run keeps ctx {i}");
        }
        assert!(!output.contains("hidden line"), "no ellipsis below threshold");
    }

    #[test]
    fn truncation_ellipsis_smallest_run_hides_two() {
        let mut children: Vec<DiffNode> = (0..7).map(|i| unchanged(&format!("ctx {i}"))).collect();
        children.push(added("new"));
        let diffs = vec![FileDiff {
            path: "m::u".into(),
            nodes: vec![DiffNode {
                name: "group".into(),
                kind: DiffKind::Modified,
                old_name: None,
                param_cases: None,
                old_param_cases: None,
                children,
            }],
        }];
        let output = format_tree(&diffs, opts_truncating());
        assert!(output.contains("... (2 hidden lines)"), "plural label for 2");
    }

    #[test]
    fn truncation_preserves_file_headers_across_files() {
        let make_file = |path: &str| FileDiff {
            path: path.into(),
            nodes: vec![DiffNode {
                name: "group".into(),
                kind: DiffKind::Modified,
                old_name: None,
                param_cases: None,
                old_param_cases: None,
                children: {
                    let mut c: Vec<DiffNode> = (0..8).map(|i| unchanged(&format!("ctx {i}"))).collect();
                    c.push(added("new"));
                    c
                },
            }],
        };
        let diffs = vec![make_file("file_a"), make_file("file_b")];
        let output = format_tree(&diffs, opts_truncating());
        assert!(output.contains("file_a"), "first file header preserved");
        assert!(output.contains("file_b"), "second file header preserved");
        assert_eq!(
            output.matches("hidden line").count(),
            2,
            "one ellipsis per file run"
        );
    }
}
