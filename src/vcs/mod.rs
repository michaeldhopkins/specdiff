pub mod git;
pub mod jj;

use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn detect(start: &Path) -> Result<Box<dyn Vcs>> {
    let (backend, root) = vcs_runner::detect_vcs(start)?;
    if backend.is_jj() {
        Ok(Box::new(jj::JjVcs::new(root)))
    } else {
        Ok(Box::new(git::GitVcs::open(&root)?))
    }
}

pub trait Vcs {
    fn changed_files(&self, base: &str, head: &str) -> Result<Vec<PathBuf>>;
    fn file_at_revision(&self, path: &Path, rev: &str) -> Result<String>;
    fn files_at_revision(&self, paths: &[PathBuf], rev: &str) -> Vec<(PathBuf, Option<String>)> {
        paths
            .iter()
            .map(|p| {
                let content = self.file_at_revision(p, rev).ok();
                (p.clone(), content)
            })
            .collect()
    }
    fn merge_base(&self, a: &str, b: &str) -> Result<String>;
    fn current_branch(&self) -> Result<Option<String>>;
    fn files_matching(&self, pattern: &str) -> Result<Vec<PathBuf>>;
    fn default_base_rev(&self) -> String;
    fn default_head_rev(&self) -> &str;
}

pub struct StubVcs {
    pub files: std::collections::HashMap<(String, String), String>,
    pub changed: Vec<PathBuf>,
    pub branch: String,
}

impl Default for StubVcs {
    fn default() -> Self {
        Self {
            files: std::collections::HashMap::new(),
            changed: vec![],
            branch: String::from("feature"),
        }
    }
}

impl Vcs for StubVcs {
    fn changed_files(&self, _base: &str, _head: &str) -> Result<Vec<PathBuf>> {
        Ok(self.changed.clone())
    }

    fn file_at_revision(&self, path: &Path, rev: &str) -> Result<String> {
        let key = (path.to_string_lossy().into_owned(), rev.to_string());
        self.files
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("file not found: {}@{}", key.0, key.1))
    }

    fn merge_base(&self, _a: &str, _b: &str) -> Result<String> {
        Ok(String::from("base"))
    }

    fn current_branch(&self) -> Result<Option<String>> {
        Ok(Some(self.branch.clone()))
    }

    fn files_matching(&self, _pattern: &str) -> Result<Vec<PathBuf>> {
        Ok(vec![])
    }

    fn default_base_rev(&self) -> String {
        "main".to_string()
    }

    fn default_head_rev(&self) -> &str {
        "HEAD"
    }
}
