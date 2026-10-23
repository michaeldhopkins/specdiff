use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub nodes: Vec<DiffNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DiffKind {
    Added,
    Removed,
    Renamed,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffNode {
    pub name: String,
    pub kind: DiffKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_cases: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_param_cases: Option<usize>,
    pub children: Vec<DiffNode>,
}

impl DiffNode {
    pub fn has_changes(&self) -> bool {
        if self.kind != DiffKind::Unchanged {
            return true;
        }
        self.children.iter().any(DiffNode::has_changes)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub added: usize,
    pub removed: usize,
    pub renamed: usize,
    pub modified: usize,
}

impl Stats {
    pub fn from_file_diffs(file_diffs: &[FileDiff]) -> Self {
        let mut stats = Stats::default();
        for fd in file_diffs {
            stats.count_nodes(&fd.nodes);
        }
        stats
    }

    fn count_nodes(&mut self, nodes: &[DiffNode]) {
        for node in nodes {
            let is_leaf = node.children.is_empty();
            match node.kind {
                DiffKind::Added if is_leaf => self.added += 1,
                DiffKind::Removed if is_leaf => self.removed += 1,
                DiffKind::Renamed => self.renamed += 1,
                DiffKind::Modified | DiffKind::Added | DiffKind::Removed | DiffKind::Unchanged => {}
            }
            self.count_nodes(&node.children);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0 && self.renamed == 0 && self.modified == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn arbitrary_diff_kind() -> impl Strategy<Value = DiffKind> {
        prop_oneof![
            Just(DiffKind::Added),
            Just(DiffKind::Removed),
            Just(DiffKind::Renamed),
            Just(DiffKind::Modified),
            Just(DiffKind::Unchanged),
        ]
    }

    fn arbitrary_diff_node() -> impl Strategy<Value = DiffNode> {
        let leaf = ("[a-z]{1,8}", arbitrary_diff_kind()).prop_map(|(name, kind)| DiffNode {
            name,
            kind,
            old_name: None,
            param_cases: None,
            old_param_cases: None,
            children: vec![],
        });
        leaf.prop_recursive(3, 16, 4, |inner| {
            ("[a-z]{1,8}", arbitrary_diff_kind(), proptest::collection::vec(inner, 0..4)).prop_map(|(name, kind, children)| DiffNode {
                name,
                kind,
                old_name: None,
                param_cases: None,
                old_param_cases: None,
                children,
            })
        })
    }

    fn arbitrary_file_diff() -> impl Strategy<Value = FileDiff> {
        ("[a-z]{1,8}", proptest::collection::vec(arbitrary_diff_node(), 0..4)).prop_map(|(path, nodes)| FileDiff { path, nodes })
    }

    fn collect_nodes<'a>(nodes: &'a [DiffNode], out: &mut Vec<&'a DiffNode>) {
        for node in nodes {
            out.push(node);
            collect_nodes(&node.children, out);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn stats_counts_added_and_removed_at_leaves_only_and_renamed_at_every_depth(
            file_diffs in proptest::collection::vec(arbitrary_file_diff(), 0..4),
        ) {
            let stats = Stats::from_file_diffs(&file_diffs);
            let mut all_nodes = Vec::new();
            for file_diff in &file_diffs {
                collect_nodes(&file_diff.nodes, &mut all_nodes);
            }
            let expected_added = all_nodes
                .iter()
                .filter(|node| node.kind == DiffKind::Added && node.children.is_empty())
                .count();
            let expected_removed = all_nodes
                .iter()
                .filter(|node| node.kind == DiffKind::Removed && node.children.is_empty())
                .count();
            let expected_renamed = all_nodes
                .iter()
                .filter(|node| node.kind == DiffKind::Renamed)
                .count();
            prop_assert_eq!(stats.added, expected_added);
            prop_assert_eq!(stats.removed, expected_removed);
            prop_assert_eq!(stats.renamed, expected_renamed);
            prop_assert_eq!(stats.modified, 0);
        }

        #[test]
        fn stats_from_file_diffs_is_additive_over_concatenation(
            left_diffs in proptest::collection::vec(arbitrary_file_diff(), 0..3),
            right_diffs in proptest::collection::vec(arbitrary_file_diff(), 0..3),
        ) {
            let mut combined_diffs = left_diffs.clone();
            combined_diffs.extend(right_diffs.clone());
            let left_stats = Stats::from_file_diffs(&left_diffs);
            let right_stats = Stats::from_file_diffs(&right_diffs);
            let combined_stats = Stats::from_file_diffs(&combined_diffs);
            prop_assert_eq!(combined_stats.added, left_stats.added + right_stats.added);
            prop_assert_eq!(combined_stats.removed, left_stats.removed + right_stats.removed);
            prop_assert_eq!(combined_stats.renamed, left_stats.renamed + right_stats.renamed);
            prop_assert_eq!(combined_stats.modified, left_stats.modified + right_stats.modified);
        }

        #[test]
        fn has_changes_is_true_iff_any_node_in_subtree_is_not_unchanged(
            node in arbitrary_diff_node(),
        ) {
            let mut subtree = Vec::new();
            collect_nodes(std::slice::from_ref(&node), &mut subtree);
            let expected = subtree.iter().any(|item| item.kind != DiffKind::Unchanged);
            prop_assert_eq!(node.has_changes(), expected);
        }

        #[test]
        fn stats_is_empty_iff_all_four_counters_are_zero(
            added in prop_oneof![Just(0usize), 1usize..100],
            removed in prop_oneof![Just(0usize), 1usize..100],
            renamed in prop_oneof![Just(0usize), 1usize..100],
            modified in prop_oneof![Just(0usize), 1usize..100],
        ) {
            let stats = Stats { added, removed, renamed, modified };
            let expected = added == 0 && removed == 0 && renamed == 0 && modified == 0;
            prop_assert_eq!(stats.is_empty(), expected);
        }
    }
}
