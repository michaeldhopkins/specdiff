use crate::cli::Cli;
use crate::diff;
use crate::diff::types::FileDiff;
use crate::parse;
use crate::parse::shared::SharedExampleRegistry;
use crate::vcs;
use anyhow::Result;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

pub trait FileSource {
    fn list_files(&self) -> Result<Vec<String>>;
    fn read_base(&self, rel_path: &str) -> Option<String>;
    fn read_head(&self, rel_path: &str) -> Option<String>;
    fn list_shared_files(&self, _glob_pattern: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    fn list_shared_files_all(&self) -> Vec<String> {
        vec![]
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

    fn list_shared_files_all(&self) -> Vec<String> {
        let mut all = std::collections::BTreeSet::new();
        for fw in parse::registry::all_frameworks() {
            if let Some(inh) = &fw.inheritance
                && inh.enabled
            {
                for glob_str in &inh.scan_globs {
                    if let Ok(pat) = glob::Pattern::new(glob_str) {
                        for dir in [&self.base, &self.head] {
                            collect_all_files_recursive(dir, dir, &pat, &mut all);
                        }
                    }
                }
            }
        }
        all.into_iter().collect()
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

    fn list_shared_files_all(&self) -> Vec<String> {
        let mut all = std::collections::BTreeSet::new();
        for fw in parse::registry::all_frameworks() {
            if let Some(inh) = &fw.inheritance
                && inh.enabled
            {
                for glob in &inh.scan_globs {
                    if let Ok(files) = self.vcs.files_matching(glob) {
                        for f in files {
                            all.insert(f.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
        all.into_iter().collect()
    }
}

pub fn diff_files(source: &dyn FileSource, cli: &Cli) -> Result<Vec<FileDiff>> {
    let all_paths = source.list_files()?;

    let needs_shared = changed_files_need_shared_scan(&all_paths, source, cli);

    let (base_registry, head_registry) = if needs_shared {
        let shared_paths = source.list_shared_files_all();
        let all_scannable: Vec<String> = {
            let mut set: std::collections::BTreeSet<String> = all_paths.iter().cloned().collect();
            set.extend(shared_paths);
            set.into_iter().collect()
        };
        (
            build_shared_registry(source, &all_scannable, cli, |s, path| s.read_base(path)),
            build_shared_registry(source, &all_scannable, cli, |s, path| s.read_head(path)),
        )
    } else {
        (
            parse::shared::SharedExampleRegistry::default(),
            parse::shared::SharedExampleRegistry::default(),
        )
    };

    diff_with_registries(source, &all_paths, cli, &base_registry, &head_registry)
}

pub fn diff_files_fast(source: &dyn FileSource, cli: &Cli) -> Result<Vec<FileDiff>> {
    let all_paths = source.list_files()?;
    let empty = SharedExampleRegistry::default();
    diff_with_registries(source, &all_paths, cli, &empty, &empty)
}

pub fn diff_files_with_registry(
    source: &dyn FileSource,
    cli: &Cli,
    registry: &SharedExampleRegistry,
) -> Result<Vec<FileDiff>> {
    let all_paths = source.list_files()?;
    diff_with_registries(source, &all_paths, cli, registry, registry)
}

struct FileContents {
    rel_path: String,
    base: Option<String>,
    head: Option<String>,
}

fn diff_with_registries(
    source: &dyn FileSource,
    all_paths: &[String],
    cli: &Cli,
    base_registry: &SharedExampleRegistry,
    head_registry: &SharedExampleRegistry,
) -> Result<Vec<FileDiff>> {
    let contents: Vec<FileContents> = all_paths
        .iter()
        .filter(|p| !parse::registry::frameworks_for_file(Path::new(p.as_str())).is_empty())
        .map(|rel_path| FileContents {
            rel_path: rel_path.clone(),
            base: source.read_base(rel_path),
            head: source.read_head(rel_path),
        })
        .collect();

    let forced_framework = cli.framework.as_deref();
    let base_shared = if base_registry.is_empty() { None } else { Some(base_registry) };
    let head_shared = if head_registry.is_empty() { None } else { Some(head_registry) };

    let use_parallel = contents.len() >= 4;

    let process = |fc: &FileContents| -> Option<FileDiff> {
        let frameworks = parse::registry::frameworks_for_file(Path::new(&fc.rel_path));
        let framework = if let Some(name) = forced_framework {
            frameworks.iter().find(|f| f.name == name).copied()
        } else {
            frameworks.first().copied()
        }?;

        let base_tree = fc.base.as_deref()
            .and_then(|s| parse::engine::parse_file_with_shared(s, &fc.rel_path, framework, base_shared));
        let head_tree = fc.head.as_deref()
            .and_then(|s| parse::engine::parse_file_with_shared(s, &fc.rel_path, framework, head_shared));

        let base_nodes = base_tree.map(|t| t.root).unwrap_or_default();
        let head_nodes = head_tree.map(|t| t.root).unwrap_or_default();

        let nodes = diff::diff_spec_nodes(&base_nodes, &head_nodes);
        if nodes.is_empty() || !nodes.iter().any(|n| n.has_changes()) {
            return None;
        }
        let display_path = parse::registry::normalize_file_path(&fc.rel_path, framework);
        Some(FileDiff { path: display_path, nodes })
    };

    let file_diffs: Vec<FileDiff> = if use_parallel {
        contents.par_iter().filter_map(process).collect()
    } else {
        contents.iter().filter_map(process).collect()
    };

    Ok(file_diffs)
}

pub fn changed_files_need_shared_scan(
    all_paths: &[String],
    source: &dyn FileSource,
    cli: &Cli,
) -> bool {
    let shared_keywords: &[&str] = &[
        "include_examples", "include_context",
        "it_behaves_like", "it_should_behave_like",
        "shared_examples", "shared_context",
    ];
    let inheritance_keywords: &[&str] = &[
        "extends", "include ", "extend ",
        "(Base", "(Test",
    ];

    for rel_path in all_paths {
        let frameworks = parse::registry::frameworks_for_file(Path::new(rel_path));
        let framework = if let Some(name) = &cli.framework {
            frameworks.iter().find(|f| f.name == *name).copied()
        } else {
            frameworks.first().copied()
        };
        let Some(fw) = framework else { continue };

        let has_shared = fw.shared.as_ref().is_some_and(|s| !s.inclusion.is_empty());
        let has_inheritance = fw.inheritance.as_ref().is_some_and(|i| i.enabled);
        if !has_shared && !has_inheritance {
            continue;
        }

        for read_fn in [FileSource::read_head, FileSource::read_base] {
            if let Some(content) = read_fn(source, rel_path) {
                if has_shared
                    && shared_keywords.iter().any(|kw| content.contains(kw))
                {
                    return true;
                }
                if has_inheritance
                    && inheritance_keywords.iter().any(|kw| content.contains(kw))
                {
                    return true;
                }
            }
        }
    }

    false
}

pub fn build_shared_registry(
    source: &dyn FileSource,
    all_paths: &[String],
    cli: &Cli,
    read_fn: impl Fn(&dyn FileSource, &str) -> Option<String>,
) -> SharedExampleRegistry {
    let mut registry = SharedExampleRegistry::default();

    let mut scan_paths: std::collections::BTreeSet<(String, &str)> = std::collections::BTreeSet::new();
    let mut framework_by_name: std::collections::HashMap<&str, &parse::registry::FrameworkDef> = std::collections::HashMap::new();

    for fw in parse::registry::all_frameworks() {
        if let Some(name) = &cli.framework
            && fw.name != *name
        {
            continue;
        }

        let has_shared_definitions = fw.shared.as_ref().is_some_and(|s| !s.definition.is_empty());
        let scans_types = fw.inheritance.as_ref().is_some_and(|i| i.enabled);

        if !has_shared_definitions && !scans_types {
            continue;
        }

        framework_by_name.insert(fw.name.as_str(), fw);

        let scan_spec_files = has_shared_definitions
            && fw.shared.as_ref().is_some_and(|s| s.scan_spec_files_for_definitions);

        if scan_spec_files || scans_types {
            for rel_path in all_paths {
                if parse::registry::frameworks_for_file(Path::new(rel_path))
                    .iter()
                    .any(|f| f.name == fw.name)
                {
                    scan_paths.insert((rel_path.clone(), fw.name.as_str()));
                }
            }
        }

        if let Some(inh) = &fw.inheritance
            && inh.enabled
        {
            for glob_pattern in &inh.scan_globs {
                if let Ok(extra_paths) = source.list_shared_files(glob_pattern) {
                    for p in extra_paths {
                        scan_paths.insert((p, fw.name.as_str()));
                    }
                }
            }
        }

        if let Some(shared_def) = &fw.shared {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSource {
        files: Vec<(String, String)>,
    }

    impl FileSource for MockSource {
        fn list_files(&self) -> Result<Vec<String>> {
            Ok(self.files.iter().map(|(name, _)| name.clone()).collect())
        }
        fn read_base(&self, _rel_path: &str) -> Option<String> {
            None
        }
        fn read_head(&self, rel_path: &str) -> Option<String> {
            self.files.iter().find(|(n, _)| n == rel_path).map(|(_, c)| c.clone())
        }
    }

    fn default_cli() -> Cli {
        Cli {
            print: false,
            base: None,
            head: None,
            format: crate::cli::OutputFormat::Tree,
            changed_only: false,
            framework: None,
            filter: None,
            no_color: true,
            base_dir: None,
            head_dir: None,
        }
    }

    #[test]
    fn skip_shared_scan_when_no_keywords() {
        let source = MockSource {
            files: vec![(
                "spec/models/user_spec.rb".into(),
                "RSpec.describe User do\n  it \"works\" do\n  end\nend\n".into(),
            )],
        };
        let cli = default_cli();
        let paths = source.list_files().expect("list");
        assert!(
            !changed_files_need_shared_scan(&paths, &source, &cli),
            "plain rspec file without shared keywords should skip scan"
        );
    }

    #[test]
    fn trigger_shared_scan_for_it_behaves_like() {
        let source = MockSource {
            files: vec![(
                "spec/models/user_spec.rb".into(),
                "RSpec.describe User do\n  it_behaves_like \"timestamped\"\nend\n".into(),
            )],
        };
        let cli = default_cli();
        let paths = source.list_files().expect("list");
        assert!(
            changed_files_need_shared_scan(&paths, &source, &cli),
            "it_behaves_like should trigger shared scan"
        );
    }

    #[test]
    fn trigger_shared_scan_for_include_examples() {
        let source = MockSource {
            files: vec![(
                "spec/models/user_spec.rb".into(),
                "RSpec.describe User do\n  include_examples \"soft delete\"\nend\n".into(),
            )],
        };
        let cli = default_cli();
        let paths = source.list_files().expect("list");
        assert!(
            changed_files_need_shared_scan(&paths, &source, &cli),
        );
    }

    #[test]
    fn trigger_shared_scan_for_python_inheritance() {
        let source = MockSource {
            files: vec![(
                "tests/test_user.py".into(),
                "class TestUser(BaseTest):\n    def test_foo(self):\n        pass\n".into(),
            )],
        };
        let cli = default_cli();
        let paths = source.list_files().expect("list");
        assert!(
            changed_files_need_shared_scan(&paths, &source, &cli),
            "(BaseTest should trigger inheritance scan"
        );
    }

    #[test]
    fn skip_shared_scan_for_plain_python() {
        let source = MockSource {
            files: vec![(
                "tests/test_user.py".into(),
                "class TestUser:\n    def test_foo(self):\n        pass\n".into(),
            )],
        };
        let cli = default_cli();
        let paths = source.list_files().expect("list");
        assert!(
            !changed_files_need_shared_scan(&paths, &source, &cli),
            "plain python class without inheritance should skip scan"
        );
    }

    #[test]
    fn skip_shared_scan_for_rust_files() {
        let source = MockSource {
            files: vec![(
                "src/lib.rs".into(),
                "#[cfg(test)]\nmod tests {\n    #[test]\n    fn test_foo() {}\n}\n".into(),
            )],
        };
        let cli = default_cli();
        let paths = source.list_files().expect("list");
        assert!(
            !changed_files_need_shared_scan(&paths, &source, &cli),
            "rust files have no shared/inheritance, should skip"
        );
    }

    #[test]
    fn diff_files_produces_output_without_shared_scan() {
        let source = MockSource {
            files: vec![(
                "spec/models/user_spec.rb".into(),
                "RSpec.describe User do\n  it \"works\" do\n  end\nend\n".into(),
            )],
        };
        let cli = default_cli();
        let diffs = diff_files(&source, &cli).expect("diff_files");
        assert!(!diffs.is_empty(), "should produce diffs even without shared scan");
    }

    #[test]
    fn parallel_diff_produces_same_results_as_serial() {
        let source = MockSource {
            files: vec![
                ("spec/models/user_spec.rb".into(), "RSpec.describe User do\n  it \"a\" do\n  end\nend\n".into()),
                ("spec/models/post_spec.rb".into(), "RSpec.describe Post do\n  it \"b\" do\n  end\nend\n".into()),
                ("spec/models/tag_spec.rb".into(), "RSpec.describe Tag do\n  it \"c\" do\n  end\nend\n".into()),
                ("spec/models/comment_spec.rb".into(), "RSpec.describe Comment do\n  it \"d\" do\n  end\nend\n".into()),
                ("spec/models/like_spec.rb".into(), "RSpec.describe Like do\n  it \"e\" do\n  end\nend\n".into()),
            ],
        };
        let cli = default_cli();
        let diffs = diff_files(&source, &cli).expect("diff_files");
        assert_eq!(diffs.len(), 5, "should diff all 5 files");
        let names: Vec<&str> = diffs.iter().map(|d| d.path.as_str()).collect();
        assert!(names.contains(&"models::user"));
        assert!(names.contains(&"models::post"));
        assert!(names.contains(&"models::tag"));
        assert!(names.contains(&"models::comment"));
        assert!(names.contains(&"models::like"));
    }

    #[test]
    fn diff_files_fast_skips_shared_resolution() {
        let source = MockSource {
            files: vec![(
                "spec/models/user_spec.rb".into(),
                "RSpec.describe User do\n  it_behaves_like \"timestamped\"\n  it \"works\" do\n  end\nend\n".into(),
            )],
        };
        let cli = default_cli();
        let diffs = diff_files_fast(&source, &cli).expect("fast");
        assert_eq!(diffs.len(), 1);
        let user = &diffs[0];
        let has_placeholder = user.nodes.iter().any(|n| {
            n.children.iter().any(|c| c.name.contains('\u{2026}'))
        });
        assert!(has_placeholder, "fast path should produce placeholder for shared inclusion");
    }

    #[test]
    fn unchanged_files_excluded_from_output() {
        let source = MockSource {
            files: vec![
                (
                    "spec/models/user_spec.rb".into(),
                    "RSpec.describe User do\n  it \"works\" do\n  end\nend\n".into(),
                ),
                (
                    "spec/models/post_spec.rb".into(),
                    "RSpec.describe Post do\n  it \"works\" do\n  end\nend\n".into(),
                ),
            ],
        };
        let cli = default_cli();
        let diffs = diff_files(&source, &cli).expect("diff_files");
        assert_eq!(
            diffs.len(), 2,
            "both files are head-only (no base), so both should appear as added"
        );

        for d in &diffs {
            assert!(
                d.nodes.iter().any(|n| n.has_changes()),
                "every file in output should have at least one change: {}",
                d.path
            );
        }
    }

    #[test]
    fn identical_base_and_head_produces_no_output() {
        let content = "RSpec.describe User do\n  it \"works\" do\n  end\nend\n".to_string();

        struct IdenticalSource(String);
        impl FileSource for IdenticalSource {
            fn list_files(&self) -> Result<Vec<String>> {
                Ok(vec!["spec/models/user_spec.rb".into()])
            }
            fn read_base(&self, _: &str) -> Option<String> {
                Some(self.0.clone())
            }
            fn read_head(&self, _: &str) -> Option<String> {
                Some(self.0.clone())
            }
        }

        let source = IdenticalSource(content);
        let cli = default_cli();
        let diffs = diff_files(&source, &cli).expect("diff_files");
        assert!(
            diffs.is_empty(),
            "identical base and head should produce no file diffs"
        );
    }

    #[test]
    fn file_with_code_changes_but_no_spec_changes_excluded() {
        let base = "RSpec.describe User do\n  it \"works\" do\n    x = 1\n  end\nend\n";
        let head = "RSpec.describe User do\n  it \"works\" do\n    x = 2\n  end\nend\n";

        struct DiffSource { base: String, head: String }
        impl FileSource for DiffSource {
            fn list_files(&self) -> Result<Vec<String>> {
                Ok(vec!["spec/models/user_spec.rb".into()])
            }
            fn read_base(&self, _: &str) -> Option<String> {
                Some(self.base.clone())
            }
            fn read_head(&self, _: &str) -> Option<String> {
                Some(self.head.clone())
            }
        }

        let source = DiffSource { base: base.into(), head: head.into() };
        let cli = default_cli();
        let diffs = diff_files(&source, &cli).expect("diff_files");
        assert!(
            diffs.is_empty(),
            "file where spec names are identical should not appear in output"
        );
    }
}
