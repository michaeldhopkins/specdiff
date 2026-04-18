use crate::diff::types::{DiffKind, DiffNode, FileDiff, Stats};
use std::fmt::Write;
use std::io::IsTerminal;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

pub fn format_json(file_diffs: &[FileDiff]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(file_diffs)?)
}

pub fn format_tree(file_diffs: &[FileDiff], changed_only: bool, color: bool) -> String {
    let use_color = color
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none();
    let mut output = String::new();

    let stats = Stats::from_file_diffs(file_diffs);
    if !stats.is_empty() {
        if use_color {
            let _ = write!(output, "{BOLD}");
        }
        let _ = write!(output, "specdiff");
        if use_color {
            let _ = write!(output, "{RESET}");
        }
        let _ = write!(output, "  ");
        if stats.added > 0 {
            let _ = write!(output, "{}", if use_color { GREEN } else { "" });
            let _ = write!(output, "+{}", stats.added);
            let _ = write!(output, "{}", if use_color { RESET } else { "" });
            let _ = write!(output, " ");
        }
        if stats.removed > 0 {
            let _ = write!(output, "{}", if use_color { RED } else { "" });
            let _ = write!(output, "-{}", stats.removed);
            let _ = write!(output, "{}", if use_color { RESET } else { "" });
            let _ = write!(output, " ");
        }
        if stats.renamed > 0 {
            let _ = write!(output, "{}", if use_color { YELLOW } else { "" });
            let _ = write!(output, "~>{}", stats.renamed);
            let _ = write!(output, "{}", if use_color { RESET } else { "" });
            let _ = write!(output, " ");
        }
        if stats.modified > 0 {
            let _ = write!(output, "{}", if use_color { CYAN } else { "" });
            let _ = write!(output, "~{}", stats.modified);
            let _ = write!(output, "{}", if use_color { RESET } else { "" });
        }
        let _ = writeln!(output);
        let _ = writeln!(output);
    }

    for file_diff in file_diffs {
        let file_has_changes = file_diff.nodes.iter().any(DiffNode::has_changes);
        if changed_only && !file_has_changes {
            continue;
        }

        if use_color {
            let _ = writeln!(output, "{BOLD}  {}{RESET}", file_diff.path);
        } else {
            let _ = writeln!(output, "  {}", file_diff.path);
        }

        for node in &file_diff.nodes {
            format_tree_node(node, &mut output, 1, changed_only, use_color);
        }
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

fn format_tree_node(node: &DiffNode, output: &mut String, depth: usize, changed_only: bool, color: bool) {
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

    let param_suffix = node
        .param_cases
        .map(|n| format!(" [{n} cases]"))
        .unwrap_or_default();

    match node.kind {
        DiffKind::Renamed => {
            if let Some(old) = &node.old_name {
                let _ = writeln!(output, "{color_start}{prefix} {indent}{old} -> {}{param_suffix}{color_end}", node.name);
            } else {
                let _ = writeln!(output, "{color_start}{prefix} {indent}{}{param_suffix}{color_end}", node.name);
            }
        }
        _ => {
            let _ = writeln!(output, "{color_start}{prefix} {indent}{}{param_suffix}{color_end}", node.name);
        }
    }

    for child in &node.children {
        format_tree_node(child, output, depth + 1, changed_only, color);
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
                let _ = writeln!(output, "-> {old} -> {full_path}");
            }
            DiffKind::Modified | DiffKind::Unchanged => {}
        }

        collect_compact_lines(&node.children, &current_path, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_file_diffs() -> Vec<FileDiff> {
        vec![FileDiff {
            path: "models::user".into(),
            nodes: vec![
                DiffNode {
                    name: "validations".into(),
                    kind: DiffKind::Modified,
                    old_name: None,
                    param_cases: None,
                    children: vec![
                        DiffNode {
                            name: "validates email".into(),
                            kind: DiffKind::Unchanged,
                            old_name: None,
                            param_cases: None,
                            children: vec![],
                        },
                        DiffNode {
                            name: "validates uniqueness".into(),
                            kind: DiffKind::Added,
                            old_name: None,
                            param_cases: None,
                            children: vec![],
                        },
                    ],
                },
                DiffNode {
                    name: "associations".into(),
                    kind: DiffKind::Unchanged,
                    old_name: None,
                    param_cases: None,
                    children: vec![DiffNode {
                        name: "has many posts".into(),
                        kind: DiffKind::Unchanged,
                        old_name: None,
                        param_cases: None,
                        children: vec![],
                    }],
                },
            ],
        }]
    }

    #[test]
    fn tree_format_shows_file_path() {
        let output = format_tree(&sample_file_diffs(), false, false);
        assert!(output.contains("models::user"), "should show file path header");
        assert!(output.contains("validations"), "should show group");
        assert!(output.contains("validates uniqueness"), "should show added spec");
    }

    #[test]
    fn tree_format_shows_stats_header() {
        let output = format_tree(&sample_file_diffs(), false, false);
        assert!(output.contains("specdiff"), "should show header");
        assert!(output.contains("+1"), "should show added count (leaf specs only)");
    }

    #[test]
    fn tree_format_changed_only() {
        let output = format_tree(&sample_file_diffs(), true, false);
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
        let output = format_tree(&sample_file_diffs(), false, false);
        assert!(!output.contains("\x1b["), "no-color output should not contain ANSI codes");
    }

    #[test]
    fn tree_format_empty_diffs_no_stats() {
        let output = format_tree(&[], false, false);
        assert!(!output.contains("+0"), "empty diffs should not show +0");
        assert!(!output.contains("-0"), "empty diffs should not show -0");
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
                children: vec![],
            }],
        }];
        let output = format_tree(&diffs, false, false);
        assert!(!output.contains("+0"), "all-unchanged should not show +0");
    }
}
