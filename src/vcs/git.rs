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
            let full_path = workdir.join(path);
            return std::fs::read_to_string(&full_path)
                .with_context(|| format!("cannot read {}", full_path.display()));
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
}
