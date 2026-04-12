pub mod registry;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum SpecKind {
    Group,
    Spec,
    SharedInclusion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParamInfo {
    pub case_count: usize,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecNode {
    pub name: String,
    pub kind: SpecKind,
    pub children: Vec<SpecNode>,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameterized: Option<ParamInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecTree {
    pub file_path: String,
    pub framework: String,
    pub root: Vec<SpecNode>,
}

impl SpecNode {
    pub fn group(name: impl Into<String>, line: usize, children: Vec<SpecNode>) -> Self {
        Self {
            name: name.into(),
            kind: SpecKind::Group,
            children,
            line,
            parameterized: None,
        }
    }

    pub fn spec(name: impl Into<String>, line: usize) -> Self {
        Self {
            name: name.into(),
            kind: SpecKind::Spec,
            children: vec![],
            line,
            parameterized: None,
        }
    }
}
