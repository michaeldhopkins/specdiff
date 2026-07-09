use crate::vcs::Vcs;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct GitVcs {
    repo: git2::Repository,
}

impl GitVcs {
    pub fn open(path: &Path) -> Result<Self> {
        let repo = git2::Repository::discover(path)
            .with_context(|| format!("no git repository found at {}", path.display()))?;
        Ok(Self { repo })
    }

    fn resolve_rev(&self, rev: &str) -> Result<git2::Oid> {
        let obj = self.repo.revparse_single(rev)
            .with_context(|| format!("cannot resolve revision '{rev}'"))?;
        Ok(obj.peel_to_commit()
            .with_context(|| format!("'{rev}' does not point to a commit"))?
            .id())
    }

    fn tree_for_commit(&self, oid: git2::Oid) -> Result<git2::Tree<'_>> {
        let commit = self.repo.find_commit(oid)?;
        Ok(commit.tree()?)
    }

    fn blob_content(&self, tree: &git2::Tree<'_>, path: &Path) -> Result<String> {
        let entry = tree.get_path(path)
            .with_context(|| format!("file {} not found in tree", path.display()))?;
        let blob = self.repo.find_blob(entry.id())
            .with_context(|| format!("cannot read blob for {}", path.display()))?;
        let content = std::str::from_utf8(blob.content())
            .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
        Ok(content.to_string())
    }
}

/// Files changed between a base commit and the on-disk working tree, via git2 —
/// no jj command, so a colocated jj repo's operation log is never touched. Diffs
/// the base tree straight against the working directory (index-independent, so a
/// jj-colocated repo's possibly-stale git index can't skew it); untracked files
/// count as additions, `.gitignore`d ones are excluded. `base_sha` is a git commit
/// id (in a colocated repo, jj's `commit_id` is exactly that).
pub(crate) fn changed_paths_to_workdir(repo_path: &Path, base_sha: &str) -> Result<Vec<PathBuf>> {
    let repo = git2::Repository::discover(repo_path)
        .with_context(|| format!("no git repository found at {}", repo_path.display()))?;
    let base_tree = repo
        .revparse_single(base_sha)
        .with_context(|| format!("cannot resolve base '{base_sha}'"))?
        .peel_to_tree()
        .with_context(|| format!("'{base_sha}' does not point to a tree"))?;

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let diff = repo.diff_tree_to_workdir(Some(&base_tree), Some(&mut opts))?;

    let mut files = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                // Defensively drop VCS-internal paths. jj's colocated `.jj/.gitignore`
                // normally hides `.jj/`, but don't rely on it: an untracked `.jj/`
                // would otherwise flood the result with false additions.
                if !path.starts_with(".jj") && !path.starts_with(".git") {
                    files.push(path.to_path_buf());
                }
            }
            true
        },
        None,
        None,
        None,
    )?;
    files.sort();
    files.dedup();
    Ok(files)
}

impl Vcs for GitVcs {
    fn changed_files(&self, base: &str, head: &str) -> Result<Vec<PathBuf>> {
        let base_oid = self.resolve_rev(base)?;
        let head_oid = self.resolve_rev(head)?;

        let base_tree = self.tree_for_commit(base_oid)?;
        let head_tree = self.tree_for_commit(head_oid)?;

        let diff = self.repo.diff_tree_to_tree(
            Some(&base_tree),
            Some(&head_tree),
            None,
        )?;

        let mut files = Vec::new();
        diff.foreach(
            &mut |delta, _| {
                if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                    files.push(path.to_path_buf());
                }
                true
            },
            None,
            None,
            None,
        )?;

        files.sort();
        files.dedup();
        Ok(files)
    }

    fn file_at_revision(&self, path: &Path, rev: &str) -> Result<String> {
        if rev == "WORKDIR" {
            let workdir = self.repo.workdir()
                .context("bare repository has no working directory")?;
            let rel = path.to_string_lossy();
            return crate::vcs::shared::read_working_file(workdir, &rel)?
                .with_context(|| format!("cannot read {} (absent from working copy)", path.display()));
        }

        let oid = self.resolve_rev(rev)?;
        let tree = self.tree_for_commit(oid)?;
        self.blob_content(&tree, path)
    }

    fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        let a_oid = self.resolve_rev(a)?;
        let b_oid = self.resolve_rev(b)?;
        let base = self.repo.merge_base(a_oid, b_oid)
            .with_context(|| format!("no merge base between '{a}' and '{b}'"))?;
        Ok(base.to_string())
    }

    fn current_branch(&self) -> Result<Option<String>> {
        let head = self.repo.head().context("HEAD is unborn")?;
        if head.is_branch() {
            return Ok(head.shorthand().map(|n| n.to_string()));
        }
        Ok(None)
    }

    fn files_matching(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let head = self.repo.head()?;
        let tree = head.peel_to_tree()?;

        let pat = glob::Pattern::new(pattern)
            .with_context(|| format!("invalid glob pattern: {pattern}"))?;

        let mut files = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if let Some(name) = entry.name() {
                let full_path = if dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{dir}{name}")
                };
                if pat.matches(&full_path) {
                    files.push(PathBuf::from(full_path));
                }
            }
            git2::TreeWalkResult::Ok
        })?;

        Ok(files)
    }

    fn default_base_rev(&self) -> String {
        for candidate in [
            "origin/main",
            "origin/master",
            "upstream/main",
            "upstream/master",
            "main",
            "master",
        ] {
            if self.resolve_rev(candidate).is_ok() {
                return candidate.to_string();
            }
        }
        "HEAD~1".to_string()
    }

    fn default_head_rev(&self) -> &str {
        "HEAD"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, GitVcs) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path();

        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .output()
            .expect("git init");

        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .expect("git config email");

        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()
            .expect("git config name");

        std::fs::create_dir_all(path.join("spec/models")).expect("mkdir");
        std::fs::write(
            path.join("spec/models/user_spec.rb"),
            "RSpec.describe User do\n  it \"exists\" do\n  end\nend\n",
        ).expect("write");

        Command::new("git")
            .args(["add", "-A"])
            .current_dir(path)
            .output()
            .expect("git add");

        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(path)
            .output()
            .expect("git commit");

        Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(path)
            .output()
            .expect("git checkout");

        std::fs::write(
            path.join("spec/models/user_spec.rb"),
            "RSpec.describe User do\n  it \"exists\" do\n  end\n  it \"validates\" do\n  end\nend\n",
        ).expect("write modified");

        Command::new("git")
            .args(["add", "-A"])
            .current_dir(path)
            .output()
            .expect("git add");

        Command::new("git")
            .args(["commit", "-m", "add validation"])
            .current_dir(path)
            .output()
            .expect("git commit");

        let vcs = GitVcs::open(path).expect("open");
        (dir, vcs)
    }

    #[test]
    fn changed_paths_to_workdir_includes_worktree_edits_and_untracked() {
        let (dir, _) = create_test_repo();
        let path = dir.path();

        // Uncommitted edit to a tracked file, plus a brand-new untracked file.
        std::fs::write(path.join("spec/models/user_spec.rb"), "changed\n").expect("write");
        std::fs::write(path.join("spec/models/order_spec.rb"), "new\n").expect("write untracked");

        let changed = changed_paths_to_workdir(path, "main").expect("changed_paths_to_workdir");

        assert!(
            changed.contains(&PathBuf::from("spec/models/user_spec.rb")),
            "tracked worktree edit must surface; got {changed:?}"
        );
        assert!(
            changed.contains(&PathBuf::from("spec/models/order_spec.rb")),
            "untracked addition must surface; got {changed:?}"
        );
    }

    #[test]
    fn changed_paths_to_workdir_clean_worktree_is_empty() {
        let (dir, _) = create_test_repo();
        // The working tree matches the committed feature branch (HEAD), so diffing
        // HEAD against the workdir yields nothing.
        let changed = changed_paths_to_workdir(dir.path(), "HEAD").expect("changed_paths_to_workdir");
        assert!(changed.is_empty(), "clean worktree must report no changes; got {changed:?}");
    }

    #[test]
    fn git_changed_files() {
        let (_dir, vcs) = create_test_repo();
        let files = vcs.changed_files("main", "feature").expect("changed_files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], PathBuf::from("spec/models/user_spec.rb"));
    }

    #[test]
    fn git_file_at_revision() {
        let (_dir, vcs) = create_test_repo();
        let content = vcs.file_at_revision(Path::new("spec/models/user_spec.rb"), "main").expect("file_at_revision");
        assert!(content.contains("exists"));
        assert!(!content.contains("validates"));

        let content = vcs.file_at_revision(Path::new("spec/models/user_spec.rb"), "feature").expect("file_at_revision");
        assert!(content.contains("validates"));
    }

    #[test]
    fn git_merge_base() {
        let (_dir, vcs) = create_test_repo();
        let base = vcs.merge_base("main", "feature").expect("merge_base");
        assert!(!base.is_empty());
    }

    #[test]
    fn git_current_branch() {
        let (_dir, vcs) = create_test_repo();
        let branch = vcs.current_branch().expect("current_branch");
        assert_eq!(branch.as_deref(), Some("feature"));
    }

    #[test]
    fn git_file_at_nonexistent_revision_errors() {
        let (_dir, vcs) = create_test_repo();
        let result = vcs.file_at_revision(Path::new("spec/models/user_spec.rb"), "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn git_file_not_in_tree_errors() {
        let (_dir, vcs) = create_test_repo();
        let result = vcs.file_at_revision(Path::new("nonexistent.rb"), "main");
        assert!(result.is_err());
    }

    #[test]
    fn default_base_rev_prefers_origin_master_over_local_master() {
        let (dir, _) = create_test_repo();

        Command::new("git")
            .args(["checkout", "-b", "master"])
            .current_dir(dir.path())
            .output()
            .expect("create local master");

        Command::new("git")
            .args(["update-ref", "refs/remotes/origin/master", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("fake origin/master ref");

        Command::new("git")
            .args(["checkout", "feature"])
            .current_dir(dir.path())
            .output()
            .expect("back to feature");

        let vcs = GitVcs::open(dir.path()).expect("reopen");
        assert_eq!(vcs.default_base_rev(), "origin/master");
    }

    #[test]
    fn default_base_rev_prefers_origin_over_upstream() {
        let (dir, _) = create_test_repo();

        Command::new("git")
            .args(["update-ref", "refs/remotes/origin/main", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("fake origin/main ref");

        Command::new("git")
            .args(["update-ref", "refs/remotes/upstream/main", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("fake upstream/main ref");

        let vcs = GitVcs::open(dir.path()).expect("reopen");
        assert_eq!(vcs.default_base_rev(), "origin/main");
    }

    #[test]
    fn default_base_rev_falls_back_to_local_when_no_remote() {
        let (dir, vcs) = create_test_repo();
        let remote_dir = dir.path().join(".git/refs/remotes");
        assert!(
            !remote_dir.exists(),
            "test repo must not have remotes for the fallback to be exercised"
        );
        assert_eq!(vcs.default_base_rev(), "main");
    }

    #[test]
    fn git_no_changes_after_merge_base_advances() {
        let (dir, _) = create_test_repo();

        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(dir.path())
            .output()
            .expect("checkout main");

        Command::new("git")
            .args(["merge", "feature", "--no-edit"])
            .current_dir(dir.path())
            .output()
            .expect("merge feature into main");

        Command::new("git")
            .args(["checkout", "feature"])
            .current_dir(dir.path())
            .output()
            .expect("checkout feature");

        let vcs = GitVcs::open(dir.path()).expect("reopen");
        let merge_base = vcs.merge_base("main", "feature").expect("merge_base");
        let files = vcs.changed_files(&merge_base, "feature").expect("changed_files");
        assert!(files.is_empty(), "after main catches up to feature, no files should be changed");
    }
}
