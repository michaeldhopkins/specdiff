use anyhow::Result;
use std::path::Path;

/// Read a working-tree file's current on-disk content, delegating to
/// `vcs_runner::read_working_file` — the churn-free way to get the working
/// ("current") side of a diff. `Ok(None)` when the file is absent (deleted).
/// Shared with branchdiff so the two apps don't drift.
pub(crate) fn read_working_file(repo_path: &Path, rel_path: &str) -> Result<Option<String>> {
    Ok(vcs_runner::read_working_file(repo_path, rel_path)?)
}
