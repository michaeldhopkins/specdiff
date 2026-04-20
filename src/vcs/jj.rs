use crate::vcs::Vcs;
use anyhow::{Context, Result};
use rayon::prelude::*;
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
        for candidate in [
            "main@origin",
            "master@origin",
            "main@upstream",
            "master@upstream",
            "main",
            "master",
        ] {
            if run_jj(&self.root, &["log", "-r", candidate, "--no-graph", "--limit", "1"])
                .is_ok()
            {
                return candidate.to_string();
            }
        }
        "trunk()".to_string()
    }

    fn files_at_revision(&self, paths: &[PathBuf], rev: &str) -> Vec<(PathBuf, Option<String>)> {
        if paths.len() < 4 {
            return paths
                .iter()
                .map(|p| (p.clone(), self.file_at_revision(p, rev).ok()))
                .collect();
        }

        let root = &self.root;
        paths
            .par_iter()
            .map(|p| {
                let path_str = p.to_string_lossy();
                let content = if rev == "WORKDIR" || rev == "@" {
                    std::fs::read_to_string(root.join(p)).ok()
                } else {
                    run_jj(root, &["file", "show", "-r", rev, &path_str])
                        .ok()
                        .map(|o| o.stdout_lossy().into_owned())
                };
                (p.clone(), content)
            })
            .collect()
    }

    fn default_head_rev(&self) -> &str {
        "@"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path();

        Command::new("git").args(["init", "-b", "main"]).current_dir(path).output().expect("git init");
        Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(path).output().expect("git config");
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(path).output().expect("git config");
        std::fs::write(path.join("README.md"), "hi\n").expect("write");
        Command::new("git").args(["add", "-A"]).current_dir(path).output().expect("git add");
        Command::new("git").args(["commit", "-m", "initial"]).current_dir(path).output().expect("git commit");

        Command::new("jj").args(["git", "init", "--colocate"]).current_dir(path).output().expect("jj init");

        dir
    }

    #[test]
    #[ignore = "requires jj CLI; run with cargo test -- --ignored"]
    fn default_base_rev_prefers_main_at_origin_over_local_main() {
        let dir = create_test_repo();

        Command::new("git")
            .args(["update-ref", "refs/remotes/origin/main", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("fake origin/main");

        Command::new("jj")
            .args(["git", "import"])
            .current_dir(dir.path())
            .output()
            .expect("jj git import");

        let vcs = JjVcs::new(dir.path().to_path_buf());
        assert_eq!(vcs.default_base_rev(), "main@origin");
    }

    #[test]
    #[ignore = "requires jj CLI; run with cargo test -- --ignored"]
    fn default_base_rev_falls_back_to_local_main_when_no_remote() {
        let dir = create_test_repo();
        let vcs = JjVcs::new(dir.path().to_path_buf());
        assert_eq!(vcs.default_base_rev(), "main");
    }

    #[test]
    #[ignore = "requires jj CLI; run with cargo test -- --ignored"]
    fn default_base_rev_prefers_origin_over_upstream() {
        let dir = create_test_repo();

        Command::new("git")
            .args(["update-ref", "refs/remotes/origin/main", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("fake origin/main");

        Command::new("git")
            .args(["update-ref", "refs/remotes/upstream/main", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("fake upstream/main");

        Command::new("jj")
            .args(["git", "import"])
            .current_dir(dir.path())
            .output()
            .expect("jj git import");

        let vcs = JjVcs::new(dir.path().to_path_buf());
        assert_eq!(vcs.default_base_rev(), "main@origin");
    }
}
