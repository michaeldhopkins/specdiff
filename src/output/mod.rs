use crate::diff::types::{DiffKind, DiffNode};
use std::fmt::Write;

pub fn format_json(nodes: &[DiffNode]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(nodes)?)
}

pub fn format_tree(nodes: &[DiffNode], changed_only: bool) -> String {
    let mut output = String::new();
    for node in nodes {
        format_tree_node(node, &mut output, 0, changed_only);
    }
    output
}

pub fn format_compact(nodes: &[DiffNode]) -> String {
    let mut output = String::new();
    collect_compact_lines(nodes, &[], &mut output);
    output
}

fn format_tree_node(node: &DiffNode, output: &mut String, depth: usize, changed_only: bool) {
    if changed_only && node.kind == DiffKind::Unchanged && !has_changes(node) {
        return;
    }

    let indent = "  ".repeat(depth);
    let prefix = match node.kind {
        DiffKind::Added => "+ ",
        DiffKind::Removed => "- ",
        DiffKind::Renamed => "->",
        DiffKind::Modified => "~ ",
        DiffKind::Unchanged => "  ",
    };

    match node.kind {
        DiffKind::Renamed => {
            if let Some(old) = &node.old_name {
                let _ = writeln!(output, "{prefix} {indent}{old} -> {}", node.name);
            } else {
                let _ = writeln!(output, "{prefix} {indent}{}", node.name);
            }
        }
        _ => {
            let _ = writeln!(output, "{prefix} {indent}{}", node.name);
        }
    }

    for child in &node.children {
        format_tree_node(child, output, depth + 1, changed_only);
    }
}

fn has_changes(node: &DiffNode) -> bool {
    if node.kind != DiffKind::Unchanged {
        return true;
    }
    node.children.iter().any(has_changes)
}

fn collect_compact_lines(nodes: &[DiffNode], path: &[&str], output: &mut String) {
    for node in nodes {
        if node.kind == DiffKind::Unchanged && !has_changes(node) {
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
    use crate::diff::types::{DiffKind, DiffNode};

    fn sample_diff() -> Vec<DiffNode> {
        vec![DiffNode {
            name: "User".into(),
            kind: DiffKind::Modified,
            old_name: None,
            children: vec![
                DiffNode {
                    name: "validations".into(),
                    kind: DiffKind::Modified,
                    old_name: None,
                    children: vec![
                        DiffNode {
                            name: "validates email".into(),
                            kind: DiffKind::Unchanged,
                            old_name: None,
                            children: vec![],
                        },
                        DiffNode {
                            name: "validates uniqueness".into(),
                            kind: DiffKind::Added,
                            old_name: None,
                            children: vec![],
                        },
                    ],
                },
                DiffNode {
                    name: "associations".into(),
                    kind: DiffKind::Unchanged,
                    old_name: None,
                    children: vec![DiffNode {
                        name: "has many posts".into(),
                        kind: DiffKind::Unchanged,
                        old_name: None,
                        children: vec![],
                    }],
                },
            ],
        }]
    }

    #[test]
    fn tree_format_shows_all() {
        let output = format_tree(&sample_diff(), false);
        assert!(output.contains("~  User"), "missing User, got:\n{output}");
        assert!(output.contains("validations"), "missing validations");
        assert!(output.contains("validates email"), "missing validates email");
        assert!(output.contains("+ "), "missing + prefix");
        assert!(output.contains("validates uniqueness"), "missing validates uniqueness");
        assert!(output.contains("associations"), "missing associations");
    }

    #[test]
    fn tree_format_changed_only() {
        let output = format_tree(&sample_diff(), true);
        assert!(output.contains("User"));
        assert!(output.contains("validations"));
        assert!(output.contains("validates uniqueness"));
        assert!(!output.contains("associations"));
    }

    #[test]
    fn compact_format() {
        let output = format_compact(&sample_diff());
        assert!(output.contains("+ User > validations > validates uniqueness"));
        assert!(!output.contains("validates email"));
        assert!(!output.contains("associations"));
    }

    #[test]
    fn json_format() {
        let diff = sample_diff();
        let json = format_json(&diff).expect("json");
        assert!(json.contains("\"Modified\""));
        assert!(json.contains("validates uniqueness"));
    }
}
