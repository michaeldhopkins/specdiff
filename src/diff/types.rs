use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub nodes: Vec<DiffNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
                DiffKind::Modified
                | DiffKind::Added
                | DiffKind::Removed
                | DiffKind::Unchanged => {}
            }
            self.count_nodes(&node.children);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0 && self.renamed == 0 && self.modified == 0
    }
}
