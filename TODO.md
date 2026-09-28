# TODO / Follow-ups

## Warm-refresh mtime stat-cache for the external-store disk walk

`JjVcs::diskwalk_changed_files` (`src/vcs/jj.rs`) is the fallback discovery path
for the rare `jj git init --git-repo=<external>` repo with no workdir `.git`. It
is correctness-first and currently re-reads O(tree) per refresh (one agnostic
`jj file show` per tracked file). The common colocated path uses git2 and never
hits this.

**Deferred optimization:** a warm-refresh mtime stat-cache (branchdiff's
`DiskManifest`) turns this into O(changed) — only files whose mtime advanced
past the last validation are re-read.

**Agreed shape when we build it:** it's the same jj-specific mechanism in both
specdiff and branchdiff, so the home is **vcs-runner's `worktree` module** as a
self-contained `jj_diskwalk_changed_paths(&mut DiskManifest, repo, base_rev)` —
resolves the base, lists it, walks the tree, reads base content on cache misses,
all working-copy-agnostic. Written and tested once; then this method and
branchdiff's diskwalk collapse to a single call and map the returned paths to
their own changed-file types.

**Tradeoff:** this pulls the `ignore` crate (ripgrep's walker) into vcs-runner,
which is otherwise a lean subprocess runner. Alternative: a small dedicated
`worktree-diff` crate depending on `ignore` + vcs-runner, keeping `ignore`
isolated. Lean toward putting it in vcs-runner unless keeping it dependency-light
is worth protecting.

**Trigger:** not worth building until a *large external-store* repo under the
live TUI actually shows the cost. The subtle bits to preserve: the racy-index
rule (`mtime >= checkpoint` must be re-read), keying the cache on the resolved
commit id (not the symbolic rev), and only persisting after a completed pass.

## `Stats.modified` is never counted

`Stats::from_file_diffs` (`src/diff/types.rs`) increments `added`, `removed` and
`renamed` but never `modified`, so the stats header's `~N` stat can never
appear and `is_empty` never sees it. Found by mutation testing (2026-09-27):
the `> 0` guard on it in `push_stats_header` has an equivalent `< 0` mutant,
excluded in `.cargo/mutants.toml` for that reason.

**Decision needed:** count modified leaves (a spec whose parameter cases
changed is the only leaf that can be `Modified`) so the header shows them, or
drop the field and its header branch. If it starts being counted, delete the
exclusion so the `< 0` mutant is tested again.
