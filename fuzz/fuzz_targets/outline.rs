#![no_main]
//! One source file, outlined by every framework its path selects.
//!
//! Input: `<path>\n<source>`. The first line picks the frameworks (see `common::frameworks_for`),
//! the rest is the file. Asserts, for each framework:
//! - outlining never panics or hangs, with or without a shared-example registry;
//! - it is deterministic: the same source outlines identically twice;
//! - diffing the outline against itself changes nothing;
//! - renaming one node (to its name plus a word) shows as exactly one rename, never as an
//!   add and a remove.

use libfuzzer_sys::fuzz_target;
use specdiff::diff::diff_spec_nodes;
use specdiff::diff::types::DiffKind;
use specdiff::parse::registry::normalize_file_path;
use specdiff::parse::{SpecKind, SpecNode};

#[path = "common.rs"]
mod common;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (path, source) = text.split_once('\n').unwrap_or((&text, ""));

    for framework in common::frameworks_for(path) {
        let _ = normalize_file_path(path, framework);
        let _ = specdiff::parse::engine::parse_file(source, path, framework);

        let first = common::outline(source, path, framework);
        let second = common::outline(source, path, framework);
        assert_eq!(first, second, "{} outlined the same source differently", framework.name);

        let same = diff_spec_nodes(&first, &first);
        assert!(
            same.iter().all(|n| !n.has_changes()),
            "{}: an outline diffed against itself shows changes: {same:?}",
            framework.name
        );
        common::assert_diff_accounts_for(&first, &first, &same);

        assert_rename_is_a_rename(&first, source.len());
    }
});

/// Every node a rename can be promised for, as a path of child indices. An empty group is
/// left out: it has no children to vouch for it, so it needs a closer name (0.7) than a
/// one-word addition gives a one-word name.
fn renameable(nodes: &[SpecNode], prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, n) in nodes.iter().enumerate() {
        prefix.push(i);
        if !(n.kind == SpecKind::Group && n.children.is_empty()) {
            out.push(prefix.clone());
        }
        if n.kind == SpecKind::Group {
            renameable(&n.children, prefix, out);
        }
        prefix.pop();
    }
}

fn assert_rename_is_a_rename(outline: &[SpecNode], pick: usize) {
    let mut candidates = Vec::new();
    renameable(outline, &mut Vec::new(), &mut candidates);
    if candidates.is_empty() {
        return;
    }
    let at = &candidates[pick % candidates.len()];

    let mut renamed = outline.to_vec();
    let (siblings, last) = {
        let mut level = &mut renamed;
        for &i in &at[..at.len() - 1] {
            level = &mut level[i].children;
        }
        (level, at[at.len() - 1])
    };
    let old = siblings[last].name.clone();
    let new = format!("{old} renamed");
    let kind = siblings[last].kind.clone();

    // Two cases where "one rename" is not the right answer, so not the promise:
    // a name with no words has nothing in common with any other name, and a node whose
    // (name, kind) is not unique among its siblings can be paired with the other copy.
    if old.split_whitespace().next().is_none() {
        return;
    }
    if siblings
        .iter()
        .enumerate()
        .any(|(i, s)| s.kind == kind && (s.name == new || (i != last && s.name == old)))
    {
        return;
    }
    siblings[last].name = new.clone();

    let diff = diff_spec_nodes(outline, &renamed);
    common::assert_diff_accounts_for(outline, &renamed, &diff);
    assert_eq!(
        (
            common::count(&diff, DiffKind::Renamed),
            common::count(&diff, DiffKind::Added),
            common::count(&diff, DiffKind::Removed),
        ),
        (1, 0, 0),
        "renaming {old:?} to {new:?} did not read as one rename: {diff:?}"
    );
}
