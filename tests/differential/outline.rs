//! specdiff's side: the outline of every test file in a project, found and parsed the way the
//! pipeline does it (the first framework `frameworks_for_file` picks, shared examples and base
//! classes registered first), minus the diff, which would lose the file path and node kinds.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use specdiff::cli::Cli;
use specdiff::parse::registry::{FrameworkDef, all_frameworks, frameworks_for_file};
use specdiff::parse::shared::SharedExampleRegistry;
use specdiff::parse::{SpecKind, SpecNode};
use specdiff::pipeline::{FileSource, build_shared_registry, changed_files_need_shared_scan};

use crate::model::TestId;

/// The suite a framework's tests are compared in. Rust's three definitions are one suite:
/// `cargo test` runs them all.
pub fn family(framework: &str) -> &str {
    match framework {
        "rust_builtin" | "rust_proptest" | "rust_rstest" => "rust",
        "go_testing" => "go",
        other => other,
    }
}

pub fn framework(name: &str) -> &'static FrameworkDef {
    all_frameworks().iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no framework {name}"))
}

/// Every file under `root` that git would see (the `ignore` walker honours `.gitignore`), as
/// `/`-separated relative paths. VCS mode lists tracked files; this is the closest a
/// directory gets.
pub fn project_files(root: &Path) -> Vec<String> {
    let mut files: Vec<String> = ignore::WalkBuilder::new(root)
        .require_git(false)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| e.path().strip_prefix(root).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
        .collect();
    files.sort();
    files
}

struct Project {
    root: PathBuf,
    all: Vec<String>,
    tests: Vec<String>,
}

impl FileSource for Project {
    fn list_files(&self) -> Result<Vec<String>> {
        Ok(self.tests.clone())
    }

    fn read_base(&self, _rel_path: &str) -> Option<String> {
        None
    }

    fn read_head(&self, rel_path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel_path)).ok()
    }

    fn list_shared_files(&self, glob_pattern: &str) -> Result<Vec<String>> {
        let pattern = glob::Pattern::new(glob_pattern)?;
        Ok(self.all.iter().filter(|f| pattern.matches(f)).cloned().collect())
    }

    fn list_shared_files_all(&self) -> Vec<String> {
        let globs: Vec<glob::Pattern> = all_frameworks()
            .iter()
            .filter_map(|f| f.inheritance.as_ref().filter(|i| i.enabled))
            .flat_map(|i| i.scan_globs.iter().filter_map(|g| glob::Pattern::new(g).ok()))
            .collect();
        self.all.iter().filter(|f| globs.iter().any(|g| g.matches(f))).cloned().collect()
    }
}

/// specdiff's outline, flattened to one identifier per leaf.
pub fn outline(root: &Path, family_name: &str) -> Vec<TestId> {
    let mut ids = Vec::new();
    for (path, nodes) in trees(root, family_name) {
        flatten(&path, &nodes, &mut Vec::new(), &mut ids);
    }
    ids
}

/// specdiff's outline of every file in `root` whose framework belongs to `family`.
pub fn trees(root: &Path, family_name: &str) -> Vec<(String, Vec<SpecNode>)> {
    let all = project_files(root);
    let tests: Vec<String> =
        all.iter().filter(|f| !frameworks_for_file(Path::new(f.as_str())).is_empty()).cloned().collect();
    let project = Project { root: root.to_path_buf(), all, tests };
    let cli = Cli::parse_from(["specdiff"]);
    let registry = if changed_files_need_shared_scan(&project.tests, &project, &cli) {
        let scannable: BTreeSet<String> =
            project.tests.iter().cloned().chain(project.list_shared_files_all()).collect();
        let scannable: Vec<String> = scannable.into_iter().collect();
        build_shared_registry(&project, &scannable, &cli, |s, p| s.read_head(p))
    } else {
        SharedExampleRegistry::default()
    };
    let shared = if registry.is_empty() { None } else { Some(&registry) };

    let mut out = Vec::new();
    for path in &project.tests {
        let Some(fw) = frameworks_for_file(Path::new(path)).first().copied() else { continue };
        if family(&fw.name) != family_name {
            continue;
        }
        let Some(source) = project.read_head(path) else { continue };
        if let Some(tree) = specdiff::parse::engine::parse_file_with_shared(&source, path, fw, shared) {
            out.push((path.clone(), tree.root));
        }
    }
    out
}

/// Leaves become identifiers. A group with no children lists no test in any framework, so
/// it contributes nothing; a group that should have had tests shows up as their absence.
pub fn flatten(file: &str, nodes: &[SpecNode], chain: &mut Vec<String>, out: &mut Vec<TestId>) {
    for node in nodes {
        chain.push(node.name.clone());
        match node.kind {
            SpecKind::Group | SpecKind::SharedInclusion if !node.children.is_empty() => {
                flatten(file, &node.children, chain, out);
            }
            SpecKind::Group => {}
            SpecKind::Spec | SpecKind::SharedInclusion => out.push(TestId {
                file: file.to_string(),
                path: chain.clone(),
                cases: node.parameterized.as_ref().map(|p| p.case_count),
            }),
        }
        chain.pop();
    }
}
