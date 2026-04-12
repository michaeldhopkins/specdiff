use crate::vcs::Vcs;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct JjVcs {
    root: PathBuf,
}

impl JjVcs {
    pub fn open(path: &Path) -> Result<Self> {
        let jj_dir = path.join(".jj");
        if !jj_dir.exists() {
            anyhow::bail!("no jj repository found at {}", path.display());
        }
        Ok(Self {
            root: path.to_path_buf(),
        })
    }

    fn run_jj(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("jj")
            .args(args)
            .current_dir(&self.root)
            .output()
            .context("failed to run jj")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("jj {} failed: {stderr}", args.join(" "));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Vcs for JjVcs {
    fn changed_files(&self, base: &str, head: &str) -> Result<Vec<PathBuf>> {
        let output = self.run_jj(&["diff", "--from", base, "--to", head, "--summary"])?;
        let mut files = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(path) = line.split_whitespace().nth(1) {
                files.push(PathBuf::from(path));
            } else if !line.starts_with(|c: char| c.is_ascii_uppercase()) {
                files.push(PathBuf::from(line));
            }
        }
        files.sort();
        files.dedup();
        Ok(files)
    }

    fn file_at_revision(&self, path: &Path, rev: &str) -> Result<String> {
        let path_str = path.to_string_lossy();
        if rev == "WORKDIR" || rev == "@" {
            let full_path = self.root.join(path);
            return std::fs::read_to_string(&full_path)
                .with_context(|| format!("cannot read {}", full_path.display()));
        }
        self.run_jj(&["file", "show", "-r", rev, &path_str])
    }

    fn merge_base(&self, _a: &str, _b: &str) -> Result<String> {
        let output = self.run_jj(&["log", "-r", "trunk()", "--no-graph", "-T", "commit_id", "--limit", "1"])?;
        let id = output.trim().to_string();
        if id.is_empty() {
            anyhow::bail!("could not determine trunk revision");
        }
        Ok(id)
    }

    fn current_branch(&self) -> Result<String> {
        let output = self.run_jj(&[
            "log", "-r", "@", "--no-graph", "-T",
            "coalesce(bookmarks, change_id.shortest())",
        ])?;
        Ok(output.trim().to_string())
    }

    fn files_matching(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let output = self.run_jj(&["file", "list"])?;
        let pat = glob::Pattern::new(pattern)
            .with_context(|| format!("invalid glob pattern: {pattern}"))?;

        Ok(output
            .lines()
            .filter(|line| pat.matches(line.trim()))
            .map(|line| PathBuf::from(line.trim()))
            .collect())
    }
}
