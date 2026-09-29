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

## Jest shared helpers never register

`frameworks/jest.toml` declares a shared definition (`ast_type =
"function_declaration"`, `detection_strategy = "contains_specs_and_exported"`,
`handler = "jest_shared_helpers"`), but `parse::shared::scan_node` only tries
DSL call nodes and matches `method_names`, which jest leaves empty, and neither
the detection strategy nor the handler is implemented. So a helper such as
`export function behavesLikeList() { it(...) }` in `__tests__/helpers/` is
never registered. Found by mutation testing (2026-09-27): the
`scan_spec_files_for_definitions` guard in `build_shared_registry` is
equivalent because of it, and is excluded in `.cargo/mutants.toml`.

**Decision needed:** implement jest helper detection, or remove the dead
config. If helpers start registering, delete that exclusion.

## proptest! tests are never outlined

`frameworks/rust_proptest.toml` declares `[[framework.property_based]]` with
`macro_name = "proptest"`, but no code reads `property_based`, and the pipeline
only ever uses the first framework `frameworks_for_file` returns, which for a
`.rs` file is `rust_builtin` (`rust_proptest` and `rust_rstest` declare no files,
so they are never selected). So a `#[test] fn` inside `proptest! { … }` is
missing from every outline. Found by the differential test (2026-09-28): `cargo
test -- --list` lists `addition_commutes`, specdiff does not. Recorded as the
`proptest! tests not outlined` class in `tests/differential/known.toml`.

**Decision needed:** parse `proptest!` token trees for `fn` items (tree-sitter
leaves a macro body as a `token_tree`, so it is a handler, not TOML), or drop
the dead config. When it is outlined, delete the known class; the harness fails
on a class that explains nothing, so it will say so.

## Other disagreements with `cargo test -- --list`

Accepted for now in `tests/differential/known.toml`, each worth a decision:

- A plain `mod` inside a `#[cfg(test)] mod` is flattened: its tests appear one
  group up. Only `#[cfg(test)]` creates a group.
- rstest `#[values]` matrices are one spec with no case count; rstest generates
  one test per combination.
- Not yet in the corpus, found by reading while fixing `#[case::name]`:
  `#[tokio::test]` (and any `path::test` attribute) is not a test marker,
  because an attribute matches on its first path segment. Add one to the corpus
  and see.

## Differential-testing trial: what is not compared yet

The differential harness (`tests/differential/`, AGENTS.md "Differential testing") compares
the outline with each framework's own listing. It is a trial of a candidate global rule
(the `rust-practices` ledger, "Differential testing against a reference implementation",
decision due 2026-10-15; the owner wants more trial data before adopting it). These
frameworks have a definition in `frameworks/` but no comparison yet. Each needs its
toolchain, a reference lister, a fixture project with one of every shape, and its accepted
differentials in `known.toml`:

- **pytest**: `pytest --collect-only -q`.
- **rspec**: `rspec --dry-run --format json`.
- **jest** and **vitest**: jest's `--listTests` names only files; use `--json` on a dry run or
  vitest's `list`.
- **junit**: needs maven, gradle or the console launcher's `--list-tests`/discovery.
- **phpunit** and **pest**: `vendor/bin/phpunit --list-tests`, `vendor/bin/pest --list-tests`.
- **exunit**: `mix test --dry-run` (needs a mix project).

Also still to run (they build real projects):

- A real Rails project's minitest suite, which needs a Rails-booting loader.
- Rust: vcs-runner and jjpr through `SPECDIFF_DIFFERENTIAL_PROJECTS`.
- Add `#[tokio::test]` to the Rust fixture corpus; reading the code suggests the outline
  misses it.
- Try expressing known differentials as predicates over source ("declared inside
  `proptest!`") instead of name globs, which turned out to be per-project.
- A cheap companion check: every field the framework TOML deserialises is read by some code.
  Two of the four bugs the trial found were fields nothing read. Both are `pub` fields in
  `src/parse/registry.rs`, which is why rustc's `dead_code` could not flag them: making the
  config types `pub(crate)` is the trial for this (the `rust-practices` ledger).
- **A differential fuzz**, not just a fixed corpus: generate test files (from a small grammar
  of test-file shapes, or by recombining pieces of real ones), run each through specdiff and
  the framework's own lister, and fail on any difference outside `known.toml`. Guide
  generation on the tuple of outcomes across the two sides, as NEZHA does, not on coverage of
  one. Its feasibility depends on the reference lister's speed per input, which the trial
  should measure:
  - Realistic for minitest (loading Ruby only) and pytest (`--collect-only` imports only):
    tens of inputs a second.
  - Marginal for Go: `-list` builds the package.
  - Impractical for Rust: every input needs a compile before `cargo test --list`.

## Release pending: 0.21.5

Unpushed on top of `main`: the properties manifest and lint-suppression ratchets, the
differential harness and CI job, and three user-facing fixes (rstest `#[case::name]`, Go
`TestMain` outlined as a test, minitest `*Test` and namespaced classes). The adversarial
review has run; it found that reading `required_param_type` dropped every Go test in a file
importing `testing` under another name (fixed before release). Still to do: bump the version
to 0.21.5 with `Cargo.lock` in sync, then push and watch the release, and the new
`differential` CI job's first run.

## Left from the 0.21.5 review

- A minitest helper class whose name ends in `Test` but is not a `Minitest::Test`
  (`class FakeClockTest; def now; end; end` in a test file) is outlined as an empty group.
  The `^Test` marker had the same reach before. The differential harness cannot see it,
  because a group with no tests lists nothing. A fix would require a superclass (or some
  tests) for a class to count; reopened classes without one would then come out flat.
- Two classes of the same name in different namespaces in one file (`UserTest` and
  `Admin::UserTest`) are sibling groups with the same name, so the diff can pair either.
  minitest names them by their full constant path; the outline shows the class only.
- Rails' `test "does a thing" do … end` (ActiveSupport's declarative `test`) is not a spec
  in `minitest.toml`, so a Rails suite written that way outlines its classes and `def
  test_` methods only. Add a `[[framework.spec]]` for `test` with a string argument, and a
  case to the minitest corpus (it needs ActiveSupport loaded, or a stub `test` class method).
- Go `func Testlower(t *testing.T)` is outlined as a test although Go says a lowercase letter
  after `Test` makes it not one. Not fixed: `go test` runs vet's `tests` check and refuses
  to build the package, so no working suite contains one.
- `known.toml` rot is checked per class. Entries that only match specdiff's own source (a
  real-project run) share a corpus class, so a stale one is never reported.
- `tests/lint_suppressions.rs` scans `src`, `tests`, `fuzz/fuzz_targets` and `examples`, not
  `build.rs` (which has no suppressions today).
