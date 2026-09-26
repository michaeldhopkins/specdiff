//! What both targets share: choosing frameworks for a fuzzed path, parsing the way the
//! pipeline does, and the invariants every outline diff must satisfy.

use std::path::Path;

use specdiff::diff::types::{DiffKind, DiffNode};
use specdiff::parse::engine::parse_file_with_shared;
use specdiff::parse::registry::{FrameworkDef, all_frameworks, frameworks_for_file};
use specdiff::parse::shared::{SharedExampleRegistry, scan_for_definitions};
use specdiff::parse::{SpecKind, SpecNode};

/// The frameworks the pipeline would consider for `path`, plus any framework that `extends`
/// one of them (`rust_proptest` and `rust_rstest` declare no files of their own, so no path
/// reaches them otherwise). A path matching nothing still gets one framework, picked by the
/// path's length, so every input reaches a parser.
pub fn frameworks_for(path: &str) -> Vec<&'static FrameworkDef> {
    let mut found = frameworks_for_file(Path::new(path));
    let extending: Vec<&'static FrameworkDef> = all_frameworks()
        .iter()
        .filter(|f| {
            f.extends
                .as_deref()
                .is_some_and(|base| found.iter().any(|b| b.name == base))
        })
        .collect();
    for f in extending {
        if !found.iter().any(|g| g.name == f.name) {
            found.push(f);
        }
    }
    if found.is_empty() {
        let all = all_frameworks();
        found.push(&all[path.len() % all.len()]);
    }
    found
}

/// Parse as the pipeline does: scan the same source for shared definitions first, and use
/// the registry only when it found something.
pub fn outline(source: &str, path: &str, framework: &FrameworkDef) -> Vec<SpecNode> {
    let mut registry = SharedExampleRegistry::default();
    scan_for_definitions(source, framework, &mut registry);
    let shared = if registry.is_empty() { None } else { Some(&registry) };
    parse_file_with_shared(source, path, framework, shared)
        .map(|t| t.root)
        .unwrap_or_default()
}

/// A name-only tree: what a diff can say about one side of the comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Named(pub String, pub Vec<Named>);

/// The diff descends only into groups (a spec or a shared inclusion is compared by name and
/// case count), so the outline side is projected the same way.
pub fn named_outline(nodes: &[SpecNode]) -> Vec<Named> {
    nodes
        .iter()
        .map(|n| {
            let children = if n.kind == SpecKind::Group {
                named_outline(&n.children)
            } else {
                vec![]
            };
            Named(n.name.clone(), children)
        })
        .collect()
}

/// The base side a diff describes: every node but the added ones, under its old name.
pub fn base_side(nodes: &[DiffNode]) -> Vec<Named> {
    nodes
        .iter()
        .filter(|n| n.kind != DiffKind::Added)
        .map(|n| {
            let name = n.old_name.clone().unwrap_or_else(|| n.name.clone());
            Named(name, base_side(&n.children))
        })
        .collect()
}

/// The head side a diff describes: every node but the removed ones.
pub fn head_side(nodes: &[DiffNode]) -> Vec<Named> {
    nodes
        .iter()
        .filter(|n| n.kind != DiffKind::Removed)
        .map(|n| Named(n.name.clone(), head_side(&n.children)))
        .collect()
}

/// Recursively sorted, for comparing sides whose order the diff does not keep: matched
/// nodes come in base order, added ones after them.
pub fn sorted(mut nodes: Vec<Named>) -> Vec<Named> {
    for n in &mut nodes {
        n.1 = sorted(std::mem::take(&mut n.1));
    }
    nodes.sort();
    nodes
}

/// Every spec on either side appears in the diff exactly once, and the kinds are coherent.
pub fn assert_diff_accounts_for(base: &[SpecNode], head: &[SpecNode], diff: &[DiffNode]) {
    assert_eq!(
        base_side(diff),
        named_outline(base),
        "the diff does not account for the base outline, in order"
    );
    assert_eq!(
        sorted(head_side(diff)),
        sorted(named_outline(head)),
        "the diff does not account for the head outline"
    );
    assert_kinds_coherent(diff);
}

fn assert_kinds_coherent(nodes: &[DiffNode]) {
    for n in nodes {
        match n.kind {
            DiffKind::Added | DiffKind::Removed => {
                assert!(n.old_name.is_none(), "{:?} node {:?} has an old name", n.kind, n.name);
                assert_all(&n.children, n.kind);
            }
            DiffKind::Unchanged => {
                assert!(!n.has_changes(), "unchanged node {:?} has changes below it", n.name);
                assert!(n.old_name.is_none() && n.old_param_cases.is_none());
            }
            DiffKind::Modified => assert!(n.old_name.is_none()),
            DiffKind::Renamed => assert!(
                n.old_name.as_ref().is_some_and(|o| *o != n.name),
                "renamed node {:?} has old name {:?}",
                n.name,
                n.old_name
            ),
        }
        assert_kinds_coherent(&n.children);
    }
}

fn assert_all(nodes: &[DiffNode], kind: DiffKind) {
    for n in nodes {
        assert_eq!(n.kind, kind, "{:?} under a {kind:?} parent", n.name);
        assert_all(&n.children, kind);
    }
}

pub fn count(nodes: &[DiffNode], kind: DiffKind) -> usize {
    nodes
        .iter()
        .map(|n| usize::from(n.kind == kind) + count(&n.children, kind))
        .sum()
}
