#![no_main]
//! Two versions of one file, diffed the way `specdiff` diffs a branch.
//!
//! Input: `<path>\0<base source>\0<head source>`, plain text on purpose: byte mutation copies
//! and tweaks spans, which is what makes the two versions share most of their material, and an
//! invariant about differences needs inputs that are mostly the same. Asserts:
//! - the whole pipeline (shared-example scan, parse, diff, path normalization, every output
//!   format, the name filter) never panics or hangs;
//! - the pipeline reports nothing for a file whose base and head are identical;
//! - the diff accounts for every node of both outlines exactly once, and its kinds are
//!   coherent (an added node holds only added nodes, an unchanged one holds no change, a
//!   rename changes the name).

use anyhow::Result;
use clap::Parser;
use libfuzzer_sys::fuzz_target;
use specdiff::cli::Cli;
use specdiff::diff::diff_spec_nodes;
use specdiff::diff::filter_file_diffs;
use specdiff::output::{TreeOptions, format_compact, format_json, format_tree};
use specdiff::pipeline::{FileSource, diff_files};

#[allow(dead_code)]
#[path = "common.rs"]
mod common;

struct Memory<'a> {
    path: &'a str,
    base: &'a str,
    head: &'a str,
}

impl FileSource for Memory<'_> {
    fn list_files(&self) -> Result<Vec<String>> {
        Ok(vec![self.path.to_string()])
    }
    fn read_base(&self, rel_path: &str) -> Option<String> {
        (rel_path == self.path).then(|| self.base.to_string())
    }
    fn read_head(&self, rel_path: &str) -> Option<String> {
        (rel_path == self.path).then(|| self.head.to_string())
    }
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '\0');
    let path = parts.next().unwrap_or("");
    let base = parts.next().unwrap_or("");
    let head = parts.next().unwrap_or("");

    let cli = Cli::parse_from(["specdiff"]);

    let unchanged = diff_files(&Memory { path, base, head: base }, &cli)
        .expect("an in-memory source cannot fail to list");
    assert!(
        unchanged.is_empty(),
        "identical base and head reported changes: {unchanged:?}"
    );

    let diffs = diff_files(&Memory { path, base, head }, &cli)
        .expect("an in-memory source cannot fail to list");
    let _ = format_tree(&diffs, TreeOptions::default());
    let _ = format_tree(&diffs, TreeOptions { changed_only: true, color: false, full_context: false });
    let _ = format_tree(&diffs, TreeOptions { changed_only: false, color: false, full_context: true });
    let _ = format_compact(&diffs);
    let _ = format_json(&diffs);
    let _ = filter_file_diffs(diffs, head.get(..3).unwrap_or(head));

    for framework in common::frameworks_for(path) {
        let base_outline = common::outline(base, path, framework);
        let head_outline = common::outline(head, path, framework);
        let diff = diff_spec_nodes(&base_outline, &head_outline);
        common::assert_diff_accounts_for(&base_outline, &head_outline, &diff);
    }
});
