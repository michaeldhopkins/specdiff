use crate::vcs::Vcs;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;
use vcs_runner::{
    is_transient_error, jj_merge_base, parse_diff_summary, parse_log_output, run_jj,
    run_jj_with_retry, run_jj_with_timeout, LOG_TEMPLATE, RunError,
};

const JJ_TIMEOUT: Duration = Duration::from_secs(30);

pub struct JjVcs {
    root: PathBuf,
}

impl JjVcs {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn open(path: &Path) -> Result<Self> {
        let jj_dir = path.join(".jj");
        if !jj_dir.exists() {
            anyhow::bail!("no jj repository found at {}", path.display());
        }
        Ok(Self { root: path.to_path_buf() })
    }
}

impl Vcs for JjVcs {
    fn changed_files(&self, base: &str, head: &str) -> Result<Vec<PathBuf>> {
        let output = run_jj_with_retry(
            &self.root,
            &["diff", "--from", base, "--to", head, "--summary"],
            is_transient_error,
        )
        .with_context(|| format!("jj diff --from {base} --to {head} --summary"))?;

        Ok(parse_diff_summary(&output.stdout_lossy())
            .into_iter()
            .map(|change| change.path)
            .collect())
    }

    fn file_at_revision(&self, path: &Path, rev: &str) -> Result<String> {
        if rev == "WORKDIR" || rev == "@" {
            let full_path = self.root.join(path);
            return std::fs::read_to_string(&full_path)
                .with_context(|| format!("cannot read {}", full_path.display()));
        }

        let path_str = path.to_string_lossy();
        match run_jj(&self.root, &["file", "show", "-r", rev, &path_str]) {
            Ok(output) => Ok(output.stdout_lossy().into_owned()),
            Err(e @ RunError::NonZeroExit { .. }) => {
                Err(anyhow::anyhow!("file {} not found at {rev}: {e}", path.display()))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        jj_merge_base(&self.root, a, b)
            .with_context(|| format!("jj merge_base {a} {b}"))?
            .ok_or_else(|| anyhow::anyhow!("no common ancestor between '{a}' and '{b}'"))
    }

    fn current_branch(&self) -> Result<Option<String>> {
        let output = run_jj_with_timeout(
            &self.root,
            &["log", "-r", "@", "--no-graph", "--template", LOG_TEMPLATE],
            JJ_TIMEOUT,
        )
        .context("jj log -r @")?;

        let result = parse_log_output(&output.stdout_lossy());
        Ok(result
            .entries
            .into_iter()
            .next()
            .and_then(|entry| entry.local_bookmarks.into_iter().next()))
    }

    fn files_matching(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let output = run_jj(&self.root, &["file", "list"]).context("jj file list")?;
        let pat = glob::Pattern::new(pattern)
            .with_context(|| format!("invalid glob pattern: {pattern}"))?;

        Ok(output
            .stdout_lossy()
            .lines()
            .filter(|line| pat.matches(line.trim()))
            .map(|line| PathBuf::from(line.trim()))
            .collect())
    }

    fn default_base_rev(&self) -> String {
        for candidate in ["main", "master"] {
            if run_jj(&self.root, &["log", "-r", candidate, "--no-graph", "--limit", "1"])
                .is_ok()
            {
                return candidate.to_string();
            }
        }
        "trunk()".to_string()
    }

    fn default_head_rev(&self) -> &str {
        "@"
    }
}
