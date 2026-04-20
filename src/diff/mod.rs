pub mod types;

use crate::parse::{SpecKind, SpecNode};
use std::cell::Cell;
use std::collections::HashMap;
use types::{DiffKind, DiffNode, FileDiff};

const NAME_WEIGHT: f64 = 0.3;
const CHILD_WEIGHT: f64 = 0.7;
const RENAME_THRESHOLD: f64 = 0.5;
const EMPTY_GROUP_NAME_THRESHOLD: f64 = 0.7;
// Cap on how many times score_candidate may recurse into diff_level for
// bottom-up rescoring across a single top-level diff_spec_nodes call.
// Typical spec trees burn a handful; this prevents pathologically nested
// all-renamed trees (O(k^(2D)) worst case) from hanging.
const RECURSIVE_PAIR_BUDGET: usize = 10_000;

enum Assignment {
    Exact(usize),
    Renamed(usize, Option<Vec<DiffNode>>),
}

struct RenameCandidate {
    bi: usize,
    hi: usize,
    score: f64,
    name_sim: f64,
    index_delta: usize,
    inner_diff: Option<Vec<DiffNode>>,
}

pub fn diff_spec_nodes(base: &[SpecNode], head: &[SpecNode]) -> Vec<DiffNode> {
    let budget = Cell::new(RECURSIVE_PAIR_BUDGET);
    diff_level(base, head, &budget)
}

fn diff_level(base: &[SpecNode], head: &[SpecNode], budget: &Cell<usize>) -> Vec<DiffNode> {
    let mut assignment: Vec<Option<Assignment>> = (0..base.len()).map(|_| None).collect();
    let mut head_matched: Vec<bool> = vec![false; head.len()];

    claim_exact_matches(base, head, &mut assignment, &mut head_matched);

    let mut candidates = collect_rename_candidates(base, head, &assignment, &head_matched, budget);
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.name_sim.total_cmp(&a.name_sim))
            .then_with(|| a.index_delta.cmp(&b.index_delta))
    });
    for c in candidates {
        if assignment[c.bi].is_some() || head_matched[c.hi] {
            continue;
        }
        assignment[c.bi] = Some(Assignment::Renamed(c.hi, c.inner_diff));
        head_matched[c.hi] = true;
    }

    build_diff_results(base, head, &mut assignment, &head_matched, budget)
}

fn claim_exact_matches(
    base: &[SpecNode],
    head: &[SpecNode],
    assignment: &mut [Option<Assignment>],
    head_matched: &mut [bool],
) {
    for (bi, b) in base.iter().enumerate() {
        for (hi, h) in head.iter().enumerate() {
            if head_matched[hi] {
                continue;
            }
            if b.name == h.name && b.kind == h.kind {
                assignment[bi] = Some(Assignment::Exact(hi));
                head_matched[hi] = true;
                break;
            }
        }
    }
}

fn collect_rename_candidates(
    base: &[SpecNode],
    head: &[SpecNode],
    assignment: &[Option<Assignment>],
    head_matched: &[bool],
    budget: &Cell<usize>,
) -> Vec<RenameCandidate> {
    let mut candidates = Vec::new();
    for bi in 0..base.len() {
        if assignment[bi].is_some() {
            continue;
        }
        for hi in 0..head.len() {
            if head_matched[hi] || base[bi].kind != head[hi].kind {
                continue;
            }
            if let Some(candidate) = score_candidate(bi, hi, base, head, budget) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn score_candidate(
    bi: usize,
    hi: usize,
    base: &[SpecNode],
    head: &[SpecNode],
    budget: &Cell<usize>,
) -> Option<RenameCandidate> {
    let name_sim = name_similarity(&base[bi].name, &head[hi].name);
    let index_delta = bi.abs_diff(hi);

    match base[bi].kind {
        SpecKind::Spec | SpecKind::SharedInclusion => {
            if name_sim >= RENAME_THRESHOLD {
                Some(RenameCandidate {
                    bi,
                    hi,
                    score: name_sim,
                    name_sim,
                    index_delta,
                    inner_diff: None,
                })
            } else {
                None
            }
        }
        SpecKind::Group => {
            if base[bi].children.is_empty() && head[hi].children.is_empty() {
                return if name_sim >= EMPTY_GROUP_NAME_THRESHOLD {
                    Some(RenameCandidate {
                        bi,
                        hi,
                        score: name_sim,
                        name_sim,
                        index_delta,
                        inner_diff: Some(vec![]),
                    })
                } else {
                    None
                };
            }

            // Phase 1 (OPT-D): cheap raw-name Dice. If it already clears the
            // threshold, bottom-up can only raise the score further, so we
            // skip recursion entirely and leave inner_diff to be computed
            // lazily if this candidate ends up winning.
            let raw_child_sim = raw_child_dice(&base[bi].children, &head[hi].children);
            let raw_score = NAME_WEIGHT * name_sim + CHILD_WEIGHT * raw_child_sim;
            if raw_score >= RENAME_THRESHOLD {
                return Some(RenameCandidate {
                    bi,
                    hi,
                    score: raw_score,
                    name_sim,
                    index_delta,
                    inner_diff: None,
                });
            }

            // Phase 2: bottom-up rescore. Recurse so that renamed children
            // count toward parent overlap (S13). Bounded by the shared
            // budget (OPT-E) so pathological all-renamed deep trees can't
            // blow up.
            if budget.get() == 0 {
                return None;
            }
            budget.set(budget.get() - 1);

            let inner = diff_level(&base[bi].children, &head[hi].children, budget);
            let bottom_up_child_sim = child_overlap(&base[bi].children, &head[hi].children, &inner);
            let bottom_up_score = NAME_WEIGHT * name_sim + CHILD_WEIGHT * bottom_up_child_sim;
            if bottom_up_score >= RENAME_THRESHOLD {
                Some(RenameCandidate {
                    bi,
                    hi,
                    score: bottom_up_score,
                    name_sim,
                    index_delta,
                    inner_diff: Some(inner),
                })
            } else {
                None
            }
        }
    }
}

fn raw_child_dice(base: &[SpecNode], head: &[SpecNode]) -> f64 {
    let total = base.len() + head.len();
    if total == 0 {
        return 0.0;
    }
    let mut base_counts: HashMap<&str, usize> = HashMap::new();
    for n in base {
        *base_counts.entry(n.name.as_str()).or_insert(0) += 1;
    }
    let intersection: usize = head
        .iter()
        .map(|n| {
            let key = n.name.as_str();
            let remaining = base_counts.entry(key).or_insert(0);
            if *remaining > 0 {
                *remaining -= 1;
                1
            } else {
                0
            }
        })
        .sum();
    2.0 * intersection as f64 / total as f64
}

fn build_diff_results(
    base: &[SpecNode],
    head: &[SpecNode],
    assignment: &mut [Option<Assignment>],
    head_matched: &[bool],
    budget: &Cell<usize>,
) -> Vec<DiffNode> {
    let mut results = Vec::with_capacity(base.len() + head.len());
    for (bi, b) in base.iter().enumerate() {
        match assignment[bi].take() {
            Some(Assignment::Exact(hi)) => results.push(build_exact_node(b, &head[hi], budget)),
            Some(Assignment::Renamed(hi, maybe_inner)) => {
                let h = &head[hi];
                let inner = maybe_inner.unwrap_or_else(|| {
                    if b.kind == SpecKind::Group {
                        diff_level(&b.children, &h.children, budget)
                    } else {
                        vec![]
                    }
                });
                results.push(build_renamed_node(b, h, inner));
            }
            None => results.push(build_removed_node(b)),
        }
    }
    for (hi, h) in head.iter().enumerate() {
        if !head_matched[hi] {
            results.push(build_added_node(h));
        }
    }
    results
}

fn build_exact_node(base: &SpecNode, head: &SpecNode, budget: &Cell<usize>) -> DiffNode {
    let inner = if base.kind == SpecKind::Group {
        diff_level(&base.children, &head.children, budget)
    } else {
        vec![]
    };
    let base_cases = param_case_count(base);
    let head_cases = param_case_count(head);
    let cases_changed = base_cases != head_cases;
    let has_changes = cases_changed || inner.iter().any(|d| d.kind != DiffKind::Unchanged);
    DiffNode {
        name: head.name.clone(),
        kind: if has_changes { DiffKind::Modified } else { DiffKind::Unchanged },
        old_name: None,
        param_cases: head_cases,
        old_param_cases: if cases_changed { base_cases } else { None },
        children: inner,
    }
}

fn build_renamed_node(base: &SpecNode, head: &SpecNode, inner: Vec<DiffNode>) -> DiffNode {
    let base_cases = param_case_count(base);
    let head_cases = param_case_count(head);
    DiffNode {
        name: head.name.clone(),
        kind: DiffKind::Renamed,
        old_name: Some(base.name.clone()),
        param_cases: head_cases,
        old_param_cases: if base_cases != head_cases { base_cases } else { None },
        children: inner,
    }
}

fn build_removed_node(base: &SpecNode) -> DiffNode {
    DiffNode {
        name: base.name.clone(),
        kind: DiffKind::Removed,
        old_name: None,
        param_cases: param_case_count(base),
        old_param_cases: None,
        children: if base.kind == SpecKind::Group {
            base.children.iter().map(make_removed).collect()
        } else {
            vec![]
        },
    }
}

fn build_added_node(head: &SpecNode) -> DiffNode {
    DiffNode {
        name: head.name.clone(),
        kind: DiffKind::Added,
        old_name: None,
        param_cases: param_case_count(head),
        old_param_cases: None,
        children: if head.kind == SpecKind::Group {
            head.children.iter().map(make_added).collect()
        } else {
            vec![]
        },
    }
}

fn param_case_count(node: &SpecNode) -> Option<usize> {
    node.parameterized.as_ref().map(|p| p.case_count)
}

fn child_overlap(base: &[SpecNode], head: &[SpecNode], inner_diff: &[DiffNode]) -> f64 {
    let total = base.len() + head.len();
    if total == 0 {
        return 0.0;
    }
    let matched = inner_diff
        .iter()
        .filter(|d| matches!(d.kind, DiffKind::Unchanged | DiffKind::Modified | DiffKind::Renamed))
        .count();
    2.0 * matched as f64 / total as f64
}

fn name_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_words: Vec<&str> = a.split_whitespace().collect();
    let b_words: Vec<&str> = b.split_whitespace().collect();

    let common = a_words.iter().filter(|w| b_words.contains(w)).count();
    let total = a_words.len().max(b_words.len());

    if total == 0 {
        return 0.0;
    }

    common as f64 / total as f64
}

pub fn filter_file_diffs(file_diffs: Vec<FileDiff>, pattern: &str) -> Vec<FileDiff> {
    let pattern_lower = pattern.to_lowercase();
    file_diffs
        .into_iter()
        .filter_map(|mut fd| {
            if fd.path.to_lowercase().contains(&pattern_lower) {
                return Some(fd);
            }
            fd.nodes = filter_nodes(fd.nodes, &pattern_lower);
            if fd.nodes.is_empty() {
                None
            } else {
                Some(fd)
            }
        })
        .collect()
}

fn filter_nodes(nodes: Vec<DiffNode>, pattern: &str) -> Vec<DiffNode> {
    nodes
        .into_iter()
        .filter_map(|mut node| {
            if node.name.to_lowercase().contains(pattern) {
                return Some(node);
            }
            node.children = filter_nodes(node.children, pattern);
            if node.children.is_empty() {
                None
            } else {
                Some(node)
            }
        })
        .collect()
}

fn make_added(node: &SpecNode) -> DiffNode {
    DiffNode {
        name: node.name.clone(),
        kind: DiffKind::Added,
        old_name: None,
        param_cases: param_case_count(node),
        old_param_cases: None,
        children: node.children.iter().map(make_added).collect(),
    }
}

fn make_removed(node: &SpecNode) -> DiffNode {
    DiffNode {
        name: node.name.clone(),
        kind: DiffKind::Removed,
        old_name: None,
        param_cases: param_case_count(node),
        old_param_cases: None,
        children: node.children.iter().map(make_removed).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::SpecNode;

    #[test]
    fn diff_identical_trees() {
        let nodes = vec![
            SpecNode::spec("validates email", 1),
            SpecNode::spec("requires password", 2),
        ];
        let diff = diff_spec_nodes(&nodes, &nodes);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| d.kind == DiffKind::Unchanged));
    }

    #[test]
    fn diff_added_spec() {
        let base = vec![SpecNode::spec("validates email", 1)];
        let head = vec![
            SpecNode::spec("validates email", 1),
            SpecNode::spec("validates uniqueness", 2),
        ];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0].kind, DiffKind::Unchanged);
        assert_eq!(diff[1].kind, DiffKind::Added);
        assert_eq!(diff[1].name, "validates uniqueness");
    }

    #[test]
    fn diff_removed_spec() {
        let base = vec![
            SpecNode::spec("validates email", 1),
            SpecNode::spec("requires password", 2),
        ];
        let head = vec![SpecNode::spec("validates email", 1)];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0].kind, DiffKind::Unchanged);
        assert_eq!(diff[1].kind, DiffKind::Removed);
        assert_eq!(diff[1].name, "requires password");
    }

    #[test]
    fn diff_renamed_spec() {
        let base = vec![SpecNode::spec("validates password presence", 1)];
        let head = vec![SpecNode::spec("validates password length", 1)];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].name, "validates password length");
        assert_eq!(diff[0].old_name.as_deref(), Some("validates password presence"));
    }

    #[test]
    fn diff_modified_group() {
        let base = vec![SpecNode::group(
            "validations",
            1,
            vec![SpecNode::spec("validates email", 2)],
        )];
        let head = vec![SpecNode::group(
            "validations",
            1,
            vec![
                SpecNode::spec("validates email", 2),
                SpecNode::spec("validates uniqueness", 3),
            ],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Modified);
        assert_eq!(diff[0].name, "validations");
        assert_eq!(diff[0].children.len(), 2);
        assert_eq!(diff[0].children[0].kind, DiffKind::Unchanged);
        assert_eq!(diff[0].children[1].kind, DiffKind::Added);
    }

    #[test]
    fn diff_added_group() {
        let base: Vec<SpecNode> = vec![];
        let head = vec![SpecNode::group(
            "admin requests",
            1,
            vec![SpecNode::spec("returns 403", 2)],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Added);
        assert_eq!(diff[0].children.len(), 1);
        assert_eq!(diff[0].children[0].kind, DiffKind::Added);
    }

    fn read_fixture(rel_path: &str) -> Option<String> {
        let fixtures_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../specdiff-tests/fixtures");
        let fixtures_path = std::path::Path::new(fixtures_dir);
        if !fixtures_path.exists() {
            eprintln!("skipping fixture test: specdiff-tests repo not found");
            return None;
        }
        let full = fixtures_path.join(rel_path);
        Some(std::fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("fixture {} should be readable: {e}", full.display())))
    }

    #[test]
    fn diff_rspec_fixture_pair() {
        use crate::parse::engine::parse_file;
        use crate::parse::registry::all_frameworks;

        let Some(base_src) = read_fixture("rspec/base/spec/models/user_spec.rb") else { return };
        let Some(head_src) = read_fixture("rspec/head/spec/models/user_spec.rb") else { return };

        let rspec = all_frameworks().iter().find(|f| f.name == "rspec").expect("rspec");
        let base_tree = parse_file(&base_src, "spec/models/user_spec.rb", rspec).expect("base");
        let head_tree = parse_file(&head_src, "spec/models/user_spec.rb", rspec).expect("head");

        let diff = diff_spec_nodes(&base_tree.root, &head_tree.root);
        assert_eq!(diff.len(), 1);

        let user = &diff[0];
        assert_eq!(user.kind, DiffKind::Modified);

        let validations = &user.children[0];
        assert_eq!(validations.kind, DiffKind::Modified);

        let has_added = validations.children.iter().any(|d| d.kind == DiffKind::Added);
        let has_renamed = validations.children.iter().any(|d| d.kind == DiffKind::Renamed);
        assert!(has_added || has_renamed, "should detect changes in validations");
    }

    #[test]
    fn name_similarity_identical() {
        assert_eq!(name_similarity("foo bar", "foo bar"), 1.0);
    }

    #[test]
    fn name_similarity_partial() {
        let sim = name_similarity("requires password", "validates password length");
        assert!(sim > 0.0);
        assert!(sim < 1.0);
    }

    #[test]
    fn name_similarity_empty() {
        assert_eq!(name_similarity("", "foo"), 0.0);
        assert_eq!(name_similarity("foo", ""), 0.0);
    }

    #[test]
    fn diff_all_new_specs() {
        let base: Vec<SpecNode> = vec![];
        let head = vec![
            SpecNode::spec("test one", 1),
            SpecNode::spec("test two", 2),
        ];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| d.kind == DiffKind::Added));
    }

    #[test]
    fn diff_all_deleted_specs() {
        let base = vec![
            SpecNode::spec("test one", 1),
            SpecNode::spec("test two", 2),
        ];
        let head: Vec<SpecNode> = vec![];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| d.kind == DiffKind::Removed));
    }

    #[test]
    fn diff_class_rename_preserved_children_detected_by_child_overlap() {
        // S5: group rename with zero name overlap but identical children.
        // With child_sim = 1.0, composite = 0.7 ≥ 0.5 → rename.
        let base = vec![SpecNode::group(
            "User",
            1,
            vec![
                SpecNode::spec("has email", 2),
                SpecNode::spec("has name", 3),
            ],
        )];
        let head = vec![SpecNode::group(
            "Account",
            1,
            vec![
                SpecNode::spec("has email", 2),
                SpecNode::spec("has name", 3),
            ],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1, "class rename with identical children should match");
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].name, "Account");
        assert_eq!(diff[0].old_name.as_deref(), Some("User"));
    }

    #[test]
    fn diff_elaborated_single_child_context_matches_as_rename() {
        // S1: the PR case — elaborated context name, 1 child each.
        // name_sim = 5/13 ≈ 0.385, child_sim = 1.0, score ≈ 0.77.
        let base = vec![SpecNode::group(
            "when user is an employee",
            1,
            vec![SpecNode::spec("permits access", 2)],
        )];
        let head = vec![SpecNode::group(
            "when user is an employee and flag is on for the record's agency",
            1,
            vec![SpecNode::spec("permits access", 2)],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert!(diff[0].old_name.as_deref().unwrap().starts_with("when user is an employee"));
    }

    #[test]
    fn diff_synonym_rename_multi_child() {
        // S2: different words, identical children.
        let base = vec![SpecNode::group(
            "when logged in",
            1,
            vec![
                SpecNode::spec("shows dashboard", 2),
                SpecNode::spec("allows edit", 3),
            ],
        )];
        let head = vec![SpecNode::group(
            "when authenticated",
            1,
            vec![
                SpecNode::spec("shows dashboard", 2),
                SpecNode::spec("allows edit", 3),
            ],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].name, "when authenticated");
        assert_eq!(diff[0].old_name.as_deref(), Some("when logged in"));
        assert_eq!(diff[0].children.len(), 2);
        assert!(diff[0].children.iter().all(|d| d.kind == DiffKind::Unchanged));
    }

    #[test]
    fn diff_rename_with_partial_child_retention() {
        // S6: one child retained, one replaced — borderline score of exactly 0.5.
        let base = vec![SpecNode::group(
            "when X",
            1,
            vec![
                SpecNode::spec("alpha", 2),
                SpecNode::spec("beta", 3),
            ],
        )];
        let head = vec![SpecNode::group(
            "when Y",
            1,
            vec![
                SpecNode::spec("alpha", 2),
                SpecNode::spec("gamma", 4),
            ],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
    }

    #[test]
    fn diff_no_child_overlap_does_not_match_as_rename() {
        // S7: shared "when" in names but zero child overlap → remove+add.
        let base = vec![SpecNode::group(
            "when X",
            1,
            vec![
                SpecNode::spec("alpha", 2),
                SpecNode::spec("beta", 3),
            ],
        )];
        let head = vec![SpecNode::group(
            "when Y",
            1,
            vec![
                SpecNode::spec("phi", 2),
                SpecNode::spec("psi", 3),
            ],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2, "disjoint children and weak name must not match");
        assert!(diff.iter().any(|d| d.kind == DiffKind::Removed));
        assert!(diff.iter().any(|d| d.kind == DiffKind::Added));
    }

    #[test]
    fn diff_ambiguous_rename_picks_highest_scoring() {
        // S9: two candidates score close; sort-by-score picks the higher one
        // (typo fix beats synonym), the other becomes Added.
        let base = vec![SpecNode::group(
            "when validating email",
            1,
            vec![SpecNode::spec("rejects invalid", 2)],
        )];
        let head = vec![
            SpecNode::group(
                "when verifying email",
                1,
                vec![SpecNode::spec("rejects invalid", 2)],
            ),
            SpecNode::group(
                "when validating the email",
                2,
                vec![SpecNode::spec("rejects invalid", 3)],
            ),
        ];
        let diff = diff_spec_nodes(&base, &head);
        let renamed = diff.iter().find(|d| d.kind == DiffKind::Renamed).expect("rename");
        assert_eq!(renamed.name, "when validating the email");
        assert_eq!(renamed.old_name.as_deref(), Some("when validating email"));
        let added = diff.iter().find(|d| d.kind == DiffKind::Added).expect("add");
        assert_eq!(added.name, "when verifying email");
    }

    #[test]
    fn diff_positional_tiebreak_pairs_by_index() {
        // S11: all four pairs score identically; positional proximity pairs
        // base[0]→head[0] and base[1]→head[1] rather than crossing.
        let base = vec![
            SpecNode::group("GET /foo", 1, vec![SpecNode::spec("returns 200", 2)]),
            SpecNode::group("GET /bar", 3, vec![SpecNode::spec("returns 200", 4)]),
        ];
        let head = vec![
            SpecNode::group("GET /foos", 1, vec![SpecNode::spec("returns 200", 2)]),
            SpecNode::group("GET /bars", 3, vec![SpecNode::spec("returns 200", 4)]),
        ];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].name, "GET /foos");
        assert_eq!(diff[0].old_name.as_deref(), Some("GET /foo"));
        assert_eq!(diff[1].kind, DiffKind::Renamed);
        assert_eq!(diff[1].name, "GET /bars");
        assert_eq!(diff[1].old_name.as_deref(), Some("GET /bar"));
    }

    #[test]
    fn diff_empty_groups_match_when_names_are_highly_similar() {
        // S12: both sides have no children; fall back to name_sim with the
        // higher empty-group threshold (0.7) to avoid stub-placeholder FPs.
        // "when the user is active" vs "when user is active" → sim = 4/5 = 0.8.
        let base = vec![SpecNode::group("when the user is active", 1, vec![])];
        let head = vec![SpecNode::group("when user is active", 1, vec![])];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].old_name.as_deref(), Some("when the user is active"));
    }

    #[test]
    fn budgeted_scoring_handles_large_fully_renamed_tree_without_hang() {
        // OPT-D+E smoke test. Build a shallow-but-wide tree (50 groups per
        // side, nothing matches by name, all children identical in name) and
        // verify it completes quickly and produces correct rename pairs. The
        // raw-Dice pre-filter short-circuits because child_sim = 1 clears
        // threshold without recursion.
        let make_tree = |prefix: &str| -> Vec<SpecNode> {
            (0..50)
                .map(|i| {
                    SpecNode::group(
                        format!("{prefix}_{i}"),
                        i,
                        vec![
                            SpecNode::spec("common child one", i * 2),
                            SpecNode::spec("common child two", i * 2 + 1),
                        ],
                    )
                })
                .collect()
        };
        let base = make_tree("before");
        let head = make_tree("after");
        let start = std::time::Instant::now();
        let diff = diff_spec_nodes(&base, &head);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "50-wide diff should be fast, took {elapsed:?}");
        assert_eq!(diff.len(), 50, "all 50 pairs should match as rename");
        assert!(diff.iter().all(|d| d.kind == DiffKind::Renamed));
    }

    #[test]
    fn stats_count_removed_leaves_after_group_expansion() {
        // OBS-1: worst-case stats delta after #6. Deleting a describe with a
        // deeply-nested subtree of 5 leaf specs should now show removed=5
        // (was removed=1 before #6 collapsed Removed groups to single lines).
        let base = vec![SpecNode::group(
            "validations",
            1,
            vec![
                SpecNode::group(
                    "email",
                    2,
                    vec![
                        SpecNode::spec("rejects blank", 3),
                        SpecNode::spec("rejects malformed", 4),
                    ],
                ),
                SpecNode::group(
                    "password",
                    5,
                    vec![
                        SpecNode::spec("requires minimum length", 6),
                        SpecNode::spec("rejects common passwords", 7),
                        SpecNode::spec("must contain special char", 8),
                    ],
                ),
            ],
        )];
        let head: Vec<SpecNode> = vec![];
        let diff = diff_spec_nodes(&base, &head);
        let file_diff = FileDiff { path: "spec/models/user_spec.rb".into(), nodes: diff };
        let stats = types::Stats::from_file_diffs(&[file_diff]);
        assert_eq!(stats.removed, 5, "each leaf under a removed group counts individually");
        assert_eq!(stats.added, 0);
        assert_eq!(stats.renamed, 0);
    }

    #[test]
    fn diff_removed_group_expands_children_symmetric_with_added() {
        // #6: Removed groups now mirror Added — their full subtree is carried
        // as Removed children, so a deletion of `describe "X" { it "a"; it "b" }`
        // shows two leaf removals rather than collapsing to a single line.
        let base = vec![SpecNode::group(
            "X",
            1,
            vec![
                SpecNode::spec("a", 2),
                SpecNode::spec("b", 3),
            ],
        )];
        let head: Vec<SpecNode> = vec![];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Removed);
        assert_eq!(diff[0].children.len(), 2, "removed group should expand its children");
        assert!(diff[0].children.iter().all(|d| d.kind == DiffKind::Removed));
    }

    #[test]
    fn diff_empty_group_vs_populated_group_does_not_match() {
        // OBS-2: one side has children, other is empty. Even with maximum
        // name_sim, score = 0.3·name_sim + 0.7·0 = 0.3 — below the 0.5
        // threshold. Must render as remove+add.
        let base = vec![SpecNode::group("describe foo bar baz", 1, vec![])];
        let head = vec![SpecNode::group(
            "describe foo bar baz",
            1,
            vec![SpecNode::spec("a", 2), SpecNode::spec("b", 3)],
        )];
        // Differing names too, so exact match can't fire either.
        let head_renamed = vec![SpecNode::group(
            "describe foo bar qux",
            1,
            vec![SpecNode::spec("a", 2), SpecNode::spec("b", 3)],
        )];
        let diff = diff_spec_nodes(&base, &head_renamed);
        assert_eq!(diff.len(), 2, "empty-vs-full must not rename-match");
        assert!(diff.iter().any(|d| d.kind == DiffKind::Removed));
        assert!(diff.iter().any(|d| d.kind == DiffKind::Added));
        // Same-name case still hits find_match first (exact), making it Modified.
        let same_diff = diff_spec_nodes(&base, &head);
        assert_eq!(same_diff.len(), 1);
        assert_eq!(same_diff[0].kind, DiffKind::Modified);
    }

    #[test]
    fn diff_empty_groups_with_only_stop_word_overlap_do_not_match() {
        // Empty-group FP guard: "when foo bar" vs "when foo baz" scores 2/3
        // ≈ 0.667, below the 0.7 empty-group threshold.
        let base = vec![SpecNode::group("when foo bar", 1, vec![])];
        let head = vec![SpecNode::group("when foo baz", 1, vec![])];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().any(|d| d.kind == DiffKind::Removed));
        assert!(diff.iter().any(|d| d.kind == DiffKind::Added));
    }

    #[test]
    fn diff_bottom_up_parent_rename_with_mixed_children() {
        // Parent is renamed AND its children are a mix of exact-matched and
        // renamed specs. Bottom-up must count both kinds toward child overlap
        // so the parent itself matches.
        let base = vec![SpecNode::group(
            "X",
            1,
            vec![
                SpecNode::spec("validates email", 2),
                SpecNode::spec("validates password", 3),
            ],
        )];
        let head = vec![SpecNode::group(
            "Y",
            1,
            vec![
                SpecNode::spec("validates email", 2),            // exact match
                SpecNode::spec("validates password presence", 3), // renamed
            ],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].old_name.as_deref(), Some("X"));
        assert_eq!(diff[0].children.len(), 2);
        assert!(diff[0].children.iter().any(|d| d.kind == DiffKind::Unchanged && d.name == "validates email"));
        assert!(diff[0].children.iter().any(|d| {
            d.kind == DiffKind::Renamed
                && d.old_name.as_deref() == Some("validates password")
                && d.name == "validates password presence"
        }));
    }

    #[test]
    fn diff_bottom_up_matches_when_children_also_renamed() {
        // S13: inner spec renamed AND outer group renamed. Bottom-up recursion
        // matches the spec first, then parent child_sim sees the renamed pair
        // as overlap and the parent itself matches.
        let base = vec![SpecNode::group(
            "inner",
            1,
            vec![SpecNode::spec("a test", 2)],
        )];
        let head = vec![SpecNode::group(
            "renamed_inner",
            1,
            vec![SpecNode::spec("an elaborated a test", 2)],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].name, "renamed_inner");
        assert_eq!(diff[0].children.len(), 1);
        assert_eq!(diff[0].children[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].children[0].name, "an elaborated a test");
    }

    #[test]
    fn diff_exact_match_flags_param_case_count_change() {
        // Same name and kind but different dataset size should now surface as
        // Modified with old_param_cases populated, not silently Unchanged.
        let base = vec![SpecNode {
            name: "validates email".into(),
            line: 1,
            kind: SpecKind::Spec,
            children: vec![],
            parameterized: Some(crate::parse::ParamInfo { case_count: 3, labels: vec![] }),
            name_is_dynamic: false,
        }];
        let head = vec![SpecNode {
            name: "validates email".into(),
            line: 1,
            kind: SpecKind::Spec,
            children: vec![],
            parameterized: Some(crate::parse::ParamInfo { case_count: 5, labels: vec![] }),
            name_is_dynamic: false,
        }];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Modified);
        assert_eq!(diff[0].param_cases, Some(5));
        assert_eq!(diff[0].old_param_cases, Some(3));
    }

    #[test]
    fn diff_renamed_spec_tracks_param_case_change() {
        let base = vec![SpecNode {
            name: "validates password presence".into(),
            line: 1,
            kind: SpecKind::Spec,
            children: vec![],
            parameterized: Some(crate::parse::ParamInfo { case_count: 2, labels: vec![] }),
            name_is_dynamic: false,
        }];
        let head = vec![SpecNode {
            name: "validates password length".into(),
            line: 1,
            kind: SpecKind::Spec,
            children: vec![],
            parameterized: Some(crate::parse::ParamInfo { case_count: 4, labels: vec![] }),
            name_is_dynamic: false,
        }];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
        assert_eq!(diff[0].param_cases, Some(4));
        assert_eq!(diff[0].old_param_cases, Some(2));
    }

    #[test]
    fn diff_unrelated_groups_with_identical_children_match_as_rename() {
        // S8-flavor: aggressive-stance acceptance. Unrelated names but identical
        // single child → matches as rename. Documented intentional tradeoff.
        let base = vec![SpecNode::group(
            "when the user is doing things that should be allowed",
            1,
            vec![SpecNode::spec("permits access", 2)],
        )];
        let head = vec![SpecNode::group(
            "when the thing is in the right state",
            1,
            vec![SpecNode::spec("permits access", 2)],
        )];
        let diff = diff_spec_nodes(&base, &head);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Renamed);
    }

    #[test]
    fn diff_empty_trees() {
        let base: Vec<SpecNode> = vec![];
        let head: Vec<SpecNode> = vec![];
        let diff = diff_spec_nodes(&base, &head);
        assert!(diff.is_empty());
    }

    #[test]
    fn filter_file_diffs_by_name() {
        let diffs = vec![
            FileDiff {
                path: "models::user".into(),
                nodes: vec![
                    DiffNode { name: "validates email".into(), kind: DiffKind::Added, old_name: None, param_cases: None, old_param_cases: None, children: vec![] },
                    DiffNode { name: "has many posts".into(), kind: DiffKind::Added, old_name: None, param_cases: None, old_param_cases: None, children: vec![] },
                ],
            },
            FileDiff {
                path: "models::post".into(),
                nodes: vec![
                    DiffNode { name: "belongs to user".into(), kind: DiffKind::Added, old_name: None, param_cases: None, old_param_cases: None, children: vec![] },
                ],
            },
        ];

        let filtered = filter_file_diffs(diffs, "email");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].path, "models::user");
        assert_eq!(filtered[0].nodes.len(), 1);
        assert_eq!(filtered[0].nodes[0].name, "validates email");
    }

    #[test]
    fn filter_file_diffs_by_path() {
        let diffs = vec![
            FileDiff {
                path: "models::user".into(),
                nodes: vec![
                    DiffNode { name: "test".into(), kind: DiffKind::Added, old_name: None, param_cases: None, old_param_cases: None, children: vec![] },
                ],
            },
            FileDiff {
                path: "models::post".into(),
                nodes: vec![
                    DiffNode { name: "test".into(), kind: DiffKind::Added, old_name: None, param_cases: None, old_param_cases: None, children: vec![] },
                ],
            },
        ];

        let filtered = filter_file_diffs(diffs, "post");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].path, "models::post");
    }

    #[test]
    fn filter_is_case_insensitive() {
        let diffs = vec![FileDiff {
            path: "models::user".into(),
            nodes: vec![
                DiffNode { name: "Validates Email".into(), kind: DiffKind::Added, old_name: None, param_cases: None, old_param_cases: None, children: vec![] },
            ],
        }];

        let filtered = filter_file_diffs(diffs, "email");
        assert_eq!(filtered.len(), 1);
    }
}
