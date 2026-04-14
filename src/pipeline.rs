use crate::cli::Cli;
use crate::diff;
use crate::diff::types::FileDiff;
use crate::parse;
use crate::parse::shared::SharedExampleRegistry;
use crate::vcs;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub trait FileSource {
    fn list_files(&self) -> Result<Vec<String>>;
    fn read_base(&self, rel_path: &str) -> Option<String>;
    fn read_head(&self, rel_path: &str) -> Option<String>;
    fn list_shared_files(&self, _glob_pattern: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
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

    fn list_shared_files(&self, glob_pattern: &str) -> Result<Vec<String>> {
        let pat = glob::Pattern::new(glob_pattern)
            .map_err(|e| anyhow::anyhow!("invalid glob: {e}"))?;
        let mut files = std::collections::BTreeSet::new();
        for dir in [&self.base, &self.head] {
            collect_all_files_recursive(dir, dir, &pat, &mut files);
        }
        Ok(files.into_iter().collect())
    }
}

fn collect_all_files_recursive(
    root: &Path,
    dir: &Path,
    pattern: &glob::Pattern,
    files: &mut std::collections::BTreeSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_all_files_recursive(root, &path, pattern, files);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel_str = rel.to_string_lossy().into_owned();
            if pattern.matches(&rel_str) {
                files.insert(rel_str);
            }
        }
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

    fn list_shared_files(&self, glob_pattern: &str) -> Result<Vec<String>> {
        self.vcs.files_matching(glob_pattern)
            .map(|files| files.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
    }
}

pub fn diff_files(source: &dyn FileSource, cli: &Cli) -> Result<Vec<FileDiff>> {
    let all_paths = source.list_files()?;

    let base_registry = build_shared_registry(source, &all_paths, cli, |s, path| s.read_base(path));
    let head_registry = build_shared_registry(source, &all_paths, cli, |s, path| s.read_head(path));

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

        let base_shared = if base_registry.is_empty() { None } else { Some(&base_registry) };
        let head_shared = if head_registry.is_empty() { None } else { Some(&head_registry) };

        let base_source = source.read_base(rel_path);
        let head_source = source.read_head(rel_path);

        let base_tree = base_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file_with_shared(s, rel_path, framework, base_shared));
        let head_tree = head_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file_with_shared(s, rel_path, framework, head_shared));

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

fn build_shared_registry(
    source: &dyn FileSource,
    all_paths: &[String],
    cli: &Cli,
    read_fn: impl Fn(&dyn FileSource, &str) -> Option<String>,
) -> SharedExampleRegistry {
    let mut registry = SharedExampleRegistry::default();

    let mut scan_paths: std::collections::BTreeSet<(String, &str)> = std::collections::BTreeSet::new();
    let mut framework_by_name: std::collections::HashMap<&str, &parse::registry::FrameworkDef> = std::collections::HashMap::new();

    for fw in parse::registry::all_frameworks() {
        if let Some(name) = &cli.framework {
            if fw.name != *name {
                continue;
            }
        }

        let shared_def = match &fw.shared {
            Some(s) if !s.definition.is_empty() => s,
            _ => continue,
        };

        framework_by_name.insert(fw.name.as_str(), fw);

        for rel_path in all_paths {
            if shared_def.scan_spec_files_for_definitions
                && parse::registry::frameworks_for_file(Path::new(rel_path))
                    .iter()
                    .any(|f| f.name == fw.name)
            {
                scan_paths.insert((rel_path.clone(), fw.name.as_str()));
            }
        }

        for glob_pattern in &shared_def.definition_globs {
            if let Ok(pat) = glob::Pattern::new(glob_pattern) {
                for rel_path in all_paths {
                    if pat.matches(rel_path) {
                        scan_paths.insert((rel_path.clone(), fw.name.as_str()));
                    }
                }
                if let Ok(extra_paths) = source.list_shared_files(glob_pattern) {
                    for p in extra_paths {
                        scan_paths.insert((p, fw.name.as_str()));
                    }
                }
            }
        }
    }

    for (rel_path, fw_name) in &scan_paths {
        let Some(fw) = framework_by_name.get(fw_name) else { continue };
        if let Some(content) = read_fn(source, rel_path) {
            parse::shared::scan_for_definitions(&content, fw, &mut registry);
        }
    }

    registry
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
