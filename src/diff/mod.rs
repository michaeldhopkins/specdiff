pub mod types;

use crate::parse::{SpecKind, SpecNode};
use types::{DiffKind, DiffNode, FileDiff};

pub fn diff_spec_nodes(base: &[SpecNode], head: &[SpecNode]) -> Vec<DiffNode> {
    let mut results = Vec::new();
    let mut matched_head: Vec<bool> = vec![false; head.len()];

    for base_node in base {
        if let Some((idx, head_node)) = find_match(base_node, head, &matched_head) {
            matched_head[idx] = true;
            let child_diff = if base_node.kind == SpecKind::Group {
                diff_spec_nodes(&base_node.children, &head_node.children)
            } else {
                vec![]
            };

            let has_changes = child_diff.iter().any(|d| d.kind != DiffKind::Unchanged);

            results.push(DiffNode {
                name: head_node.name.clone(),
                kind: if has_changes { DiffKind::Modified } else { DiffKind::Unchanged },
                old_name: None,
                children: child_diff,
            });
        } else if let Some((idx, head_node)) = find_rename_candidate(base_node, head, &matched_head) {
            matched_head[idx] = true;
            let child_diff = if base_node.kind == SpecKind::Group {
                diff_spec_nodes(&base_node.children, &head_node.children)
            } else {
                vec![]
            };
            results.push(DiffNode {
                name: head_node.name.clone(),
                kind: DiffKind::Renamed,
                old_name: Some(base_node.name.clone()),
                children: child_diff,
            });
        } else {
            results.push(DiffNode {
                name: base_node.name.clone(),
                kind: DiffKind::Removed,
                old_name: None,
                children: vec![],
            });
        }
    }

    for (idx, head_node) in head.iter().enumerate() {
        if !matched_head[idx] {
            results.push(DiffNode {
                name: head_node.name.clone(),
                kind: DiffKind::Added,
                old_name: None,
                children: if head_node.kind == SpecKind::Group {
                    head_node.children.iter().map(make_added).collect()
                } else {
                    vec![]
                },
            });
        }
    }

    results
}

fn find_match<'a>(
    base_node: &SpecNode,
    head: &'a [SpecNode],
    matched: &[bool],
) -> Option<(usize, &'a SpecNode)> {
    for (idx, head_node) in head.iter().enumerate() {
        if !matched[idx] && base_node.name == head_node.name && base_node.kind == head_node.kind {
            return Some((idx, head_node));
        }
    }
    None
}

fn find_rename_candidate<'a>(
    base_node: &SpecNode,
    head: &'a [SpecNode],
    matched: &[bool],
) -> Option<(usize, &'a SpecNode)> {
    if base_node.kind != SpecKind::Spec {
        return None;
    }

    let mut best: Option<(usize, &SpecNode, f64)> = None;

    for (idx, head_node) in head.iter().enumerate() {
        if matched[idx] || head_node.kind != base_node.kind {
            continue;
        }
        let sim = name_similarity(&base_node.name, &head_node.name);
        if sim >= 0.5
            && best.as_ref().is_none_or(|(_, _, best_sim)| sim > *best_sim)
        {
            best = Some((idx, head_node, sim));
        }
    }

    best.map(|(idx, node, _)| (idx, node))
}

fn name_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_words: Vec<&str> = a.split_whitespace().collect();
    let b_words: Vec<&str> = b.split_whitespace().collect();

    let common = a_words.iter().filter(|w| b_words.contains(w)).count();
    let total = a_words.len().max(b_words.len());

    if total == 0 {
        return 0.0;
    }

    common as f64 / total as f64
}

pub fn filter_file_diffs(file_diffs: Vec<FileDiff>, pattern: &str) -> Vec<FileDiff> {
    let pattern_lower = pattern.to_lowercase();
    file_diffs
        .into_iter()
        .filter_map(|mut fd| {
            if fd.path.to_lowercase().contains(&pattern_lower) {
                return Some(fd);
            }
            fd.nodes = filter_nodes(fd.nodes, &pattern_lower);
            if fd.nodes.is_empty() {
                None
            } else {
                Some(fd)
            }
        })
        .collect()
}

fn filter_nodes(nodes: Vec<DiffNode>, pattern: &str) -> Vec<DiffNode> {
    nodes
        .into_iter()
        .filter_map(|mut node| {
            if node.name.to_lowercase().contains(pattern) {
                return Some(node);
            }
            node.children = filter_nodes(node.children, pattern);
            if node.children.is_empty() {
                None
            } else {
                Some(node)
            }
        })
        .collect()
}

fn make_added(node: &SpecNode) -> DiffNode {
    DiffNode {
        name: node.name.clone(),
        kind: DiffKind::Added,
        old_name: None,
        children: node.children.iter().map(make_added).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::SpecNode;

    #[test]
    fn diff_identical_trees() {
        let nodes = vec![
            SpecNode::spec("validates email", 1),
            SpecNode::spec("requires password", 2),
        ];
        let diff = diff_spec_nodes(&nodes, &nodes);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| d.kind == DiffKind::Unchanged));
    }

    #[test]
    fn diff_added_spec() {
        let base = vec![SpecNode::spec("validates email", 1)];
        let head = vec![
            SpecNode::spec("validates email", 1),
            SpecNode::spec("validates uniqueness", 2),
        ];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0].kind, DiffKind::Unchanged);
        assert_eq!(diff[1].kind, DiffKind::Added);
        assert_eq!(diff[1].name, "validates uniqueness");
    }

    #[test]
    fn diff_removed_spec() {
        let base = vec![
            SpecNode::spec("validates email", 1),
            SpecNode::spec("requires password", 2),
        ];
        let head = vec![SpecNode::spec("validates email", 1)];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0].kind, DiffKind::Unchanged);
        assert_eq!(diff[1].kind, DiffKind::Removed);
        assert_eq!(diff[1].name, "requires password");
    }

    #[test]
    fn diff_renamed_spec() {
        let base = vec![SpecNode::spec("validates password presence", 1)];
        let head = vec![SpecNode::spec("validates password length", 1)];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].name, "validates password length");
        assert_eq!(diff[0].old_name.as_deref(), Some("validates password presence"));
    }

    #[test]
    fn diff_modified_group() {
        let base = vec![SpecNode::group(
            "validations",
            1,
            vec![SpecNode::spec("validates email", 2)],
        )];
        let head = vec![SpecNode::group(
            "validations",
            1,
            vec![
                SpecNode::spec("validates email", 2),
                SpecNode::spec("validates uniqueness", 3),
            ],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Modified);
        assert_eq!(diff[0].name, "validations");
        assert_eq!(diff[0].children.len(), 2);
        assert_eq!(diff[0].children[0].kind, DiffKind::Unchanged);
        assert_eq!(diff[0].children[1].kind, DiffKind::Added);
    }

    #[test]
    fn diff_added_group() {
        let base: Vec<SpecNode> = vec![];
        let head = vec![SpecNode::group(
            "admin requests",
            1,
            vec![SpecNode::spec("returns 403", 2)],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Added);
        assert_eq!(diff[0].children.len(), 1);
        assert_eq!(diff[0].children[0].kind, DiffKind::Added);
    }

    fn read_fixture(rel_path: &str) -> Option<String> {
        let fixtures_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../specdiff-tests/fixtures");
        let fixtures_path = std::path::Path::new(fixtures_dir);
        if !fixtures_path.exists() {
            eprintln!("skipping fixture test: specdiff-tests repo not found");
            return None;
        }
        let full = fixtures_path.join(rel_path);
        Some(std::fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("fixture {} should be readable: {e}", full.display())))
    }

    #[test]
    fn diff_rspec_fixture_pair() {
        use crate::parse::engine::parse_file;
        use crate::parse::registry::all_frameworks;

        let Some(base_src) = read_fixture("rspec/base/spec/models/user_spec.rb") else { return };
        let Some(head_src) = read_fixture("rspec/head/spec/models/user_spec.rb") else { return };

        let rspec = all_frameworks().iter().find(|f| f.name == "rspec").expect("rspec");
        let base_tree = parse_file(&base_src, "spec/models/user_spec.rb", rspec).expect("base");
        let head_tree = parse_file(&head_src, "spec/models/user_spec.rb", rspec).expect("head");

        let diff = diff_spec_nodes(&base_tree.root, &head_tree.root);
        assert_eq!(diff.len(), 1);

        let user = &diff[0];
        assert_eq!(user.kind, DiffKind::Modified);

        let validations = &user.children[0];
        assert_eq!(validations.kind, DiffKind::Modified);

        let has_added = validations.children.iter().any(|d| d.kind == DiffKind::Added);
        let has_renamed = validations.children.iter().any(|d| d.kind == DiffKind::Renamed);
        assert!(has_added || has_renamed, "should detect changes in validations");
    }

    #[test]
    fn name_similarity_identical() {
        assert_eq!(name_similarity("foo bar", "foo bar"), 1.0);
    }

    #[test]
    fn name_similarity_partial() {
        let sim = name_similarity("requires password", "validates password length");
        assert!(sim > 0.0);
        assert!(sim < 1.0);
    }

    #[test]
    fn name_similarity_empty() {
        assert_eq!(name_similarity("", "foo"), 0.0);
        assert_eq!(name_similarity("foo", ""), 0.0);
    }

    #[test]
    fn diff_all_new_specs() {
        let base: Vec<SpecNode> = vec![];
        let head = vec![
            SpecNode::spec("test one", 1),
            SpecNode::spec("test two", 2),
        ];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| d.kind == DiffKind::Added));
    }

    #[test]
    fn diff_all_deleted_specs() {
        let base = vec![
            SpecNode::spec("test one", 1),
            SpecNode::spec("test two", 2),
        ];
        let head: Vec<SpecNode> = vec![];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| d.kind == DiffKind::Removed));
    }

    #[test]
    fn diff_renamed_group_shows_as_removed_and_added() {
        let base = vec![SpecNode::group(
            "User",
            1,
            vec![SpecNode::spec("has name", 2)],
        )];
        let head = vec![SpecNode::group(
            "Account",
            1,
            vec![SpecNode::spec("has name", 2)],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2, "renamed group should be removed + added, not renamed");
        assert_eq!(diff[0].kind, DiffKind::Removed);
        assert_eq!(diff[0].name, "User");
        assert_eq!(diff[1].kind, DiffKind::Added);
        assert_eq!(diff[1].name, "Account");
    }

    #[test]
    fn diff_empty_trees() {
        let base: Vec<SpecNode> = vec![];
        let head: Vec<SpecNode> = vec![];
        let diff = diff_spec_nodes(&base, &head);
        assert!(diff.is_empty());
    }

    #[test]
    fn filter_file_diffs_by_name() {
        let diffs = vec![
            FileDiff {
                path: "models::user".into(),
                nodes: vec![
                    DiffNode { name: "validates email".into(), kind: DiffKind::Added, old_name: None, children: vec![] },
                    DiffNode { name: "has many posts".into(), kind: DiffKind::Added, old_name: None, children: vec![] },
                ],
            },
            FileDiff {
                path: "models::post".into(),
                nodes: vec![
                    DiffNode { name: "belongs to user".into(), kind: DiffKind::Added, old_name: None, children: vec![] },
                ],
            },
        ];

        let filtered = filter_file_diffs(diffs, "email");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].path, "models::user");
        assert_eq!(filtered[0].nodes.len(), 1);
        assert_eq!(filtered[0].nodes[0].name, "validates email");
    }

    #[test]
    fn filter_file_diffs_by_path() {
        let diffs = vec![
            FileDiff {
                path: "models::user".into(),
                nodes: vec![
                    DiffNode { name: "test".into(), kind: DiffKind::Added, old_name: None, children: vec![] },
                ],
            },
            FileDiff {
                path: "models::post".into(),
                nodes: vec![
                    DiffNode { name: "test".into(), kind: DiffKind::Added, old_name: None, children: vec![] },
                ],
            },
        ];

        let filtered = filter_file_diffs(diffs, "post");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].path, "models::post");
    }

    #[test]
    fn filter_is_case_insensitive() {
        let diffs = vec![FileDiff {
            path: "models::user".into(),
            nodes: vec![
                DiffNode { name: "Validates Email".into(), kind: DiffKind::Added, old_name: None, children: vec![] },
            ],
        }];

        let filtered = filter_file_diffs(diffs, "email");
        assert_eq!(filtered.len(), 1);
    }
}
