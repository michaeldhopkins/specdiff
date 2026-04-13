use anyhow::{Context, Result};
use clap::Parser;
use spec_diff::cli;
use spec_diff::diff;
use spec_diff::diff::types::FileDiff;
use spec_diff::output;
use spec_diff::parse;
use spec_diff::tui;
use spec_diff::vcs;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    if cli.watch {
        return tui::run_watch(&cli);
    }

    match (&cli.base_dir, &cli.head_dir) {
        (Some(base_dir), Some(head_dir)) => run_directory_diff(base_dir, head_dir, &cli),
        (Some(_), None) | (None, Some(_)) => {
            anyhow::bail!("--base-dir and --head-dir must be used together")
        }
        (None, None) => run_vcs_diff(&cli),
    }
}


trait FileSource {
    fn list_files(&self) -> Result<Vec<String>>;
    fn read_base(&self, rel_path: &str) -> Option<String>;
    fn read_head(&self, rel_path: &str) -> Option<String>;
}

struct DirectorySource {
    base: PathBuf,
    head: PathBuf,
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

struct VcsSource<'a> {
    vcs: &'a dyn vcs::Vcs,
    files: Vec<PathBuf>,
    merge_base: String,
    head_rev: String,
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

fn diff_files(source: &dyn FileSource, cli: &cli::Cli) -> Result<Vec<FileDiff>> {
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

fn run_vcs_diff(cli: &cli::Cli) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine working directory")?;
    let vcs = vcs::detect(&cwd)?;

    let branch = vcs.current_branch()?;
    let base_rev = cli.base.clone().unwrap_or_else(|| "main".to_string());
    let head_rev = cli.head.clone().unwrap_or_else(|| vcs.default_head_rev().to_string());

    let merge_base = vcs.merge_base(&base_rev, &head_rev)
        .unwrap_or_else(|_| base_rev.clone());

    let changed = vcs.changed_files(&merge_base, &head_rev)?;

    let test_files: Vec<PathBuf> = changed
        .into_iter()
        .filter(|f| !parse::registry::frameworks_for_file(f).is_empty())
        .collect();

    if test_files.is_empty() {
        eprintln!("No test file changes detected on {branch}.");
        return Ok(());
    }

    let source = VcsSource {
        vcs: vcs.as_ref(),
        files: test_files,
        merge_base,
        head_rev,
    };

    let file_diffs = diff_files(&source, cli)?;
    render_output(&file_diffs, cli)
}

fn run_directory_diff(base_dir: &str, head_dir: &str, cli: &cli::Cli) -> Result<()> {
    let source = DirectorySource {
        base: PathBuf::from(base_dir),
        head: PathBuf::from(head_dir),
    };

    let file_diffs = diff_files(&source, cli)?;
    render_output(&file_diffs, cli)
}

fn render_output(file_diffs: &[FileDiff], cli: &cli::Cli) -> Result<()> {
    let file_diffs = if let Some(pattern) = &cli.filter {
        diff::filter_file_diffs(file_diffs.to_vec(), pattern)
    } else {
        file_diffs.to_vec()
    };
    let file_diffs = &file_diffs;

    let output_str = match cli.format {
        cli::OutputFormat::Tree => output::format_tree(file_diffs, cli.changed_only, !cli.no_color),
        cli::OutputFormat::Json => output::format_json(file_diffs)
            .context("failed to serialize JSON")?,
        cli::OutputFormat::Compact => output::format_compact(file_diffs),
    };

    print!("{output_str}");
    Ok(())
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
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, files)?;
        } else if let Some(rel) = pathdiff(root, &path) {
            if !parse::registry::frameworks_for_file(Path::new(&rel)).is_empty() {
                files.push(rel);
            }
        }
    }
    Ok(())
}

fn pathdiff(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}
