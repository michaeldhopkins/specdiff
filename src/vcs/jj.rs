use crate::vcs::{is_working_copy_rev, Vcs};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Duration;
use vcs_runner::{
    is_transient_error, parse_diff_summary, parse_log_output, run_jj, run_jj_utf8_ignore_wc,
    run_jj_with_retry, run_jj_with_timeout, LOG_TEMPLATE, RunError,
};

const JJ_TIMEOUT: Duration = Duration::from_secs(30);

/// Prepend `--ignore-working-copy` so a read never snapshots the user's
/// in-progress edits. specdiff is a read-only observer that runs on a timer and
/// on filesystem events in watch mode; an incidental snapshot there races any
/// foreground `jj` and can fork the operation log. specdiff never snapshots — it
/// discovers working-copy changes against disk — so every jj read goes through this.
fn ignore_wc<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut full = Vec::with_capacity(args.len() + 1);
    full.push("--ignore-working-copy");
    full.extend_from_slice(args);
    full
}

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

    /// A colocated repo keeps a workdir `.git` beside `.jj`, so git can observe the
    /// working tree without running any jj command (and churning its oplog).
    fn is_colocated(&self) -> bool {
        self.root.join(".git").exists()
    }

    /// Resolve a jj revision to its git commit id, working-copy-agnostically. In a
    /// colocated repo this is exactly the git sha of the revision, which git2 can
    /// then resolve. `None` if the rev doesn't resolve.
    fn commit_id_of(&self, rev: &str) -> Option<String> {
        run_jj_utf8_ignore_wc(
            &self.root,
            &["log", "-r", rev, "--no-graph", "--limit", "1", "-T", "commit_id"],
        )
        .ok()
        .filter(|id| !id.is_empty())
    }

    /// Files changed between `base` and the on-disk working copy (`@` plus live,
    /// un-snapshotted edits) WITHOUT snapshotting. Colocated repos read the worktree
    /// through git2; the rare external-store repo (no workdir `.git`) walks the disk
    /// against the base tree. Reading the working side from disk is what keeps
    /// specdiff a churn-free observer even as it refreshes on every save.
    fn discover_working_changes(&self, base: &str) -> Result<Vec<PathBuf>> {
        // Prefer git2 in a colocated repo; fall back to the (also churn-free) disk
        // walk if git can't resolve the base — e.g. a conflicted or not-yet-exported
        // jj commit — rather than failing the refresh.
        if self.is_colocated()
            && let Some(base_sha) = self.commit_id_of(base)
            && let Ok(changed) = crate::vcs::git::changed_paths_to_workdir(&self.root, &base_sha)
        {
            return Ok(changed);
        }
        self.diskwalk_changed_files(base)
    }

    /// The base tree's tracked file list, read working-copy-agnostically.
    fn base_file_list(&self, base: &str) -> Result<Vec<String>> {
        let out = run_jj_utf8_ignore_wc(&self.root, &["file", "list", "-r", base])?;
        Ok(out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect())
    }

    /// Fallback discovery for repos with no workdir git index (the rare
    /// `jj git init --git-repo=<external>` form): compare the base tree to the
    /// on-disk working tree directly — no git, no snapshot. Correctness-first: this
    /// uncommon path re-reads O(tree) per refresh; the common colocated path uses
    /// git2 and never hits it. A warm-refresh mtime cache is a deferred follow-up
    /// (see TODO.md).
    fn diskwalk_changed_files(&self, base: &str) -> Result<Vec<PathBuf>> {
        let base_files = self.base_file_list(base)?;
        let base_set: std::collections::HashSet<&str> =
            base_files.iter().map(String::as_str).collect();

        let mut changed: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in ignore::WalkBuilder::new(&self.root)
            .hidden(false)
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                name != ".jj" && name != ".git"
            })
            .build()
            .flatten()
        {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&self.root) else { continue };
            let path = rel.to_string_lossy().replace('\\', "/");
            seen.insert(path.clone());

            let disk = crate::vcs::shared::read_working_file(&self.root, &path).ok().flatten();
            let base_content = if base_set.contains(path.as_str()) {
                run_jj(&self.root, &ignore_wc(&["file", "show", "-r", base, &path]))
                    .ok()
                    .map(|o| o.stdout_lossy().into_owned())
            } else {
                None
            };
            if disk.as_deref() != base_content.as_deref() {
                changed.push(PathBuf::from(&path));
            }
        }

        // Deletions: base files no longer present on disk.
        for bp in &base_files {
            if !seen.contains(bp.as_str()) {
                changed.push(PathBuf::from(bp));
            }
        }
        changed.sort();
        changed.dedup();
        Ok(changed)
    }
}

impl Vcs for JjVcs {
    fn changed_files(&self, base: &str, head: &str) -> Result<Vec<PathBuf>> {
        // The working copy (head = @) is discovered against disk without ever
        // snapshotting; a committed head is a churn-free agnostic commit-to-commit
        // diff (no working copy involved, so nothing to snapshot).
        if is_working_copy_rev(head) {
            return self.discover_working_changes(base);
        }
        let output = run_jj_with_retry(
            &self.root,
            &ignore_wc(&["diff", "--from", base, "--to", head, "--summary"]),
            is_transient_error,
        )
        .with_context(|| format!("jj diff --from {base} --to {head} --summary"))?;

        Ok(parse_diff_summary(&output.stdout_lossy())
            .into_iter()
            .map(|change| change.path)
            .collect())
    }

    fn file_at_revision(&self, path: &Path, rev: &str) -> Result<String> {
        if is_working_copy_rev(rev) {
            let rel = path.to_string_lossy();
            return crate::vcs::shared::read_working_file(&self.root, &rel)?
                .with_context(|| format!("cannot read {} (absent from working copy)", path.display()));
        }

        let path_str = path.to_string_lossy();
        match run_jj(&self.root, &ignore_wc(&["file", "show", "-r", rev, &path_str])) {
            Ok(output) => Ok(output.stdout_lossy().into_owned()),
            Err(e @ RunError::NonZeroExit { .. }) => {
                Err(anyhow::anyhow!("file {} not found at {rev}: {e}", path.display()))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        // Ancestry only — no working copy needed. Agnostic so the watch-mode tick
        // that polls this every few seconds never snapshots the user's edits.
        // (vcs-runner's `jj_merge_base` is not agnostic, so we issue the query.)
        let revset = format!("latest(::({a}) & ::({b}))");
        let id = run_jj_utf8_ignore_wc(
            &self.root,
            &["log", "-r", &revset, "--no-graph", "--limit", "1", "-T", "commit_id"],
        )
        .with_context(|| format!("jj merge_base {a} {b}"))?;
        if id.is_empty() {
            anyhow::bail!("no common ancestor between '{a}' and '{b}'");
        }
        Ok(id)
    }

    fn current_branch(&self) -> Result<Option<String>> {
        let output = run_jj_with_timeout(
            &self.root,
            &ignore_wc(&["log", "-r", "@", "--no-graph", "--template", LOG_TEMPLATE]),
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
        let output = run_jj(&self.root, &ignore_wc(&["file", "list"])).context("jj file list")?;
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
            if run_jj(&self.root, &ignore_wc(&["log", "-r", candidate, "--no-graph", "--limit", "1"]))
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
                let content = if is_working_copy_rev(rev) {
                    crate::vcs::shared::read_working_file(root, &path_str).ok().flatten()
                } else {
                    run_jj(root, &ignore_wc(&["file", "show", "-r", rev, &path_str]))
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

    /// The working-copy commit id, read agnostically so measuring never itself
    /// snapshots. When jj folds a dirty edit into `@`, this id changes.
    fn wc_commit_id(path: &Path) -> String {
        let out = Command::new("jj")
            .args(["--ignore-working-copy", "log", "-r", "@", "--no-graph", "-T", "commit_id"])
            .current_dir(path)
            .output()
            .expect("jj log @");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    #[ignore = "requires jj CLI; run with cargo test -- --ignored"]
    fn idle_reads_do_not_snapshot_working_copy() {
        let dir = create_test_repo();
        let vcs = JjVcs::new(dir.path().to_path_buf());

        // Dirty the working copy without snapshotting it.
        std::fs::write(dir.path().join("README.md"), "uncommitted edit\n").expect("write");
        let before = wc_commit_id(dir.path());

        // Every read — including the working-copy diff — must leave `@` untouched;
        // specdiff never snapshots.
        let _ = vcs.changed_files("@", "@");
        let _ = vcs.merge_base("@", "@");
        let _ = vcs.current_branch();
        let _ = vcs.default_base_rev();
        let _ = vcs.files_matching("*.md");

        assert_eq!(before, wc_commit_id(dir.path()), "idle reads must not snapshot the working copy");
    }

    /// Number of entries in the jj operation log, read agnostically. A snapshot of
    /// the working copy would add one; a churn-free refresh leaves it unchanged.
    fn op_count(path: &Path) -> usize {
        let out = Command::new("jj")
            .args(["--ignore-working-copy", "op", "log", "--no-graph", "-T", r#"id ++ "\n""#])
            .current_dir(path)
            .output()
            .expect("jj op log");
        String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).count()
    }

    #[test]
    #[ignore = "requires jj CLI; run with cargo test -- --ignored"]
    fn refresh_does_not_churn_the_oplog() {
        let dir = create_test_repo();
        let vcs = JjVcs::new(dir.path().to_path_buf());

        // A brand-new, un-snapshotted test file in the colocated working tree.
        std::fs::write(
            dir.path().join("user_spec.rb"),
            "RSpec.describe User do\n  it 'new' do\n  end\nend\n",
        )
        .expect("write");

        let ops_before = op_count(dir.path());
        let changed = vcs.changed_files("@-", "@").expect("changed_files");
        let ops_after = op_count(dir.path());

        assert_eq!(ops_before, ops_after, "refresh must not create a jj operation");
        assert!(
            changed.iter().any(|p| p == Path::new("user_spec.rb")),
            "refresh must surface the un-snapshotted edit; got {changed:?}"
        );
    }

    #[test]
    #[ignore = "requires jj CLI; run with cargo test -- --ignored"]
    fn diskwalk_surfaces_edits_without_churn() {
        let dir = create_test_repo();
        let vcs = JjVcs::new(dir.path().to_path_buf());

        std::fs::write(
            dir.path().join("user_spec.rb"),
            "RSpec.describe User do\n  it 'new' do\n  end\nend\n",
        )
        .expect("write");

        let ops_before = op_count(dir.path());
        // Drive the external-store fallback directly (no workdir git required).
        let changed = vcs.diskwalk_changed_files("@-").expect("diskwalk");
        let ops_after = op_count(dir.path());

        assert_eq!(ops_before, ops_after, "diskwalk must not create a jj operation");
        assert!(
            changed.iter().any(|p| p == Path::new("user_spec.rb")),
            "diskwalk must surface the un-snapshotted edit; got {changed:?}"
        );
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
