use crate::cli::Cli;
use crate::diff;
use crate::diff::types::FileDiff;
use crate::parse;
use crate::vcs;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub trait FileSource {
    fn list_files(&self) -> Result<Vec<String>>;
    fn read_base(&self, rel_path: &str) -> Option<String>;
    fn read_head(&self, rel_path: &str) -> Option<String>;
}

pub struct DirectorySource {
    pub base: PathBuf,
    pub head: PathBuf,
}

impl FileSource for DirectorySource {
    fn list_files(&self) -> Result<Vec<String>> {
        let base_files = collect_test_files(&self.base)?;
        let head_files = collect_test_files(&self.head)?;
        let all: std::collections::BTreeSet<String> = base_files
            .into_iter()
            .chain(head_files)
            .collect();
        Ok(all.into_iter().collect())
    }

    fn read_base(&self, rel_path: &str) -> Option<String> {
        std::fs::read_to_string(self.base.join(rel_path)).ok()
    }

    fn read_head(&self, rel_path: &str) -> Option<String> {
        std::fs::read_to_string(self.head.join(rel_path)).ok()
    }
}

pub struct VcsSource<'a> {
    pub vcs: &'a dyn vcs::Vcs,
    pub files: Vec<PathBuf>,
    pub merge_base: String,
    pub head_rev: String,
}

impl FileSource for VcsSource<'_> {
    fn list_files(&self) -> Result<Vec<String>> {
        Ok(self.files.iter().map(|p| p.to_string_lossy().into_owned()).collect())
    }

    fn read_base(&self, rel_path: &str) -> Option<String> {
        self.vcs.file_at_revision(Path::new(rel_path), &self.merge_base).ok()
    }

    fn read_head(&self, rel_path: &str) -> Option<String> {
        self.vcs.file_at_revision(Path::new(rel_path), &self.head_rev).ok()
    }
}

pub fn diff_files(source: &dyn FileSource, cli: &Cli) -> Result<Vec<FileDiff>> {
    let all_paths = source.list_files()?;
    let mut file_diffs = Vec::new();

    for rel_path in &all_paths {
        let frameworks = parse::registry::frameworks_for_file(Path::new(rel_path));
        let framework = if let Some(name) = &cli.framework {
            frameworks.iter().find(|f| f.name == *name).copied()
        } else {
            frameworks.first().copied()
        };

        let Some(framework) = framework else {
            continue;
        };

        let base_source = source.read_base(rel_path);
        let head_source = source.read_head(rel_path);

        let base_tree = base_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, rel_path, framework));
        let head_tree = head_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, rel_path, framework));

        let base_nodes = base_tree.map(|t| t.root).unwrap_or_default();
        let head_nodes = head_tree.map(|t| t.root).unwrap_or_default();

        let nodes = diff::diff_spec_nodes(&base_nodes, &head_nodes);
        if !nodes.is_empty() {
            let display_path = parse::registry::normalize_file_path(rel_path, framework);
            file_diffs.push(FileDiff { path: display_path, nodes });
        }
    }

    Ok(file_diffs)
}

fn collect_test_files(dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    collect_files_recursive(dir, dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_recursive(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, files)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel_str = rel.to_string_lossy().into_owned();
            if !parse::registry::frameworks_for_file(Path::new(&rel_str)).is_empty() {
                files.push(rel_str);
            }
        }
    }
    Ok(())
}
