#![allow(dead_code)]

mod cli;
mod diff;
mod output;
mod parse;
mod vcs;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    if let (Some(base_dir), Some(head_dir)) = (&cli.base_dir, &cli.head_dir) {
        return run_directory_diff(base_dir, head_dir, &cli);
    }

    run_vcs_diff(&cli)
}

fn detect_vcs(start: &Path) -> Result<Box<dyn vcs::Vcs>> {
    if start.join(".jj").exists() {
        let jj = vcs::jj::JjVcs::open(start)?;
        return Ok(Box::new(jj));
    }

    let git = vcs::git::GitVcs::open(start)?;
    Ok(Box::new(git))
}

fn run_vcs_diff(cli: &cli::Cli) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine working directory")?;
    let vcs = detect_vcs(&cwd)?;

    let branch = vcs.current_branch()?;
    let base_rev = cli.base.clone().unwrap_or_else(|| "main".to_string());
    let head_rev = cli.head.clone().unwrap_or_else(|| "HEAD".to_string());

    let merge_base = vcs.merge_base(&base_rev, &head_rev)
        .unwrap_or_else(|_| base_rev.clone());

    let changed = vcs.changed_files(&merge_base, &head_rev)?;

    let test_files: Vec<&PathBuf> = changed
        .iter()
        .filter(|f| !parse::registry::frameworks_for_file(f).is_empty())
        .collect();

    if test_files.is_empty() {
        eprintln!("No test file changes detected on {branch}.");
        return Ok(());
    }

    let mut all_diff_nodes = Vec::new();

    for path in &test_files {
        let frameworks = parse::registry::frameworks_for_file(path);
        let framework = if let Some(name) = &cli.framework {
            frameworks.iter().find(|f| f.name == *name).copied()
        } else {
            frameworks.first().copied()
        };

        let Some(framework) = framework else {
            continue;
        };

        let base_source = vcs.file_at_revision(path, &merge_base).ok();
        let head_source = vcs.file_at_revision(path, &head_rev).ok();

        let rel_path = path.to_string_lossy();
        let base_tree = base_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, &rel_path, framework));
        let head_tree = head_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, &rel_path, framework));

        let base_nodes = base_tree.map(|t| t.root).unwrap_or_default();
        let head_nodes = head_tree.map(|t| t.root).unwrap_or_default();

        let file_diff = diff::diff_spec_nodes(&base_nodes, &head_nodes);
        if !file_diff.is_empty() {
            all_diff_nodes.extend(file_diff);
        }
    }

    let output_str = match cli.format {
        cli::OutputFormat::Tree => output::format_tree(&all_diff_nodes, cli.changed_only, !cli.no_color),
        cli::OutputFormat::Json => output::format_json(&all_diff_nodes)
            .context("failed to serialize JSON")?,
        cli::OutputFormat::Compact => output::format_compact(&all_diff_nodes),
    };

    print!("{output_str}");
    Ok(())
}

fn run_directory_diff(base_dir: &str, head_dir: &str, cli: &cli::Cli) -> Result<()> {
    let base_path = Path::new(base_dir);
    let head_path = Path::new(head_dir);

    let base_files = collect_test_files(base_path)?;
    let head_files = collect_test_files(head_path)?;

    let all_rel_paths: std::collections::BTreeSet<String> = base_files
        .iter()
        .chain(head_files.iter())
        .cloned()
        .collect();

    let mut all_diff_nodes = Vec::new();

    for rel_path in &all_rel_paths {
        let frameworks = parse::registry::frameworks_for_file(Path::new(rel_path));
        let framework = if let Some(name) = &cli.framework {
            frameworks.iter().find(|f| f.name == *name).copied()
        } else {
            frameworks.first().copied()
        };

        let Some(framework) = framework else {
            continue;
        };

        let base_source = read_file_if_exists(&base_path.join(rel_path));
        let head_source = read_file_if_exists(&head_path.join(rel_path));

        let base_tree = base_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, rel_path, framework));
        let head_tree = head_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, rel_path, framework));

        let base_nodes = base_tree.map(|t| t.root).unwrap_or_default();
        let head_nodes = head_tree.map(|t| t.root).unwrap_or_default();

        let file_diff = diff::diff_spec_nodes(&base_nodes, &head_nodes);

        if !file_diff.is_empty() {
            all_diff_nodes.extend(file_diff);
        }
    }

    let output_str = match cli.format {
        cli::OutputFormat::Tree => output::format_tree(&all_diff_nodes, cli.changed_only, !cli.no_color),
        cli::OutputFormat::Json => output::format_json(&all_diff_nodes)
            .context("failed to serialize JSON")?,
        cli::OutputFormat::Compact => output::format_compact(&all_diff_nodes),
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

fn read_file_if_exists(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}
