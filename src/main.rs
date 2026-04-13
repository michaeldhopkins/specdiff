use anyhow::{Context, Result};
use clap::Parser;
use spec_diff::cli;
use spec_diff::diff;
use spec_diff::output;
use spec_diff::parse;
use spec_diff::pipeline::{self, DirectorySource, VcsSource};
use spec_diff::tui;
use spec_diff::vcs;
use std::path::PathBuf;

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

    let file_diffs = pipeline::diff_files(&source, cli)?;
    render_output(&file_diffs, cli)
}

fn run_directory_diff(base_dir: &str, head_dir: &str, cli: &cli::Cli) -> Result<()> {
    let source = DirectorySource {
        base: PathBuf::from(base_dir),
        head: PathBuf::from(head_dir),
    };

    let file_diffs = pipeline::diff_files(&source, cli)?;
    render_output(&file_diffs, cli)
}

fn render_output(file_diffs: &[diff::types::FileDiff], cli: &cli::Cli) -> Result<()> {
    let file_diffs = if let Some(pattern) = &cli.filter {
        diff::filter_file_diffs(file_diffs.to_vec(), pattern)
    } else {
        file_diffs.to_vec()
    };

    let output_str = match cli.format {
        cli::OutputFormat::Tree => output::format_tree(&file_diffs, cli.changed_only, !cli.no_color),
        cli::OutputFormat::Json => output::format_json(&file_diffs)
            .context("failed to serialize JSON")?,
        cli::OutputFormat::Compact => output::format_compact(&file_diffs),
    };

    print!("{output_str}");
    Ok(())
}
