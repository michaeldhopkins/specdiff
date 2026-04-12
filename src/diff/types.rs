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
