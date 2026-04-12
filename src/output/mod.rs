use crate::diff::types::DiffNode;

pub fn format_json(nodes: &[DiffNode]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(nodes)?)
}
