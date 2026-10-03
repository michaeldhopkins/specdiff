# AGENTS.md

## Reference projects

All of my projects are available in ~/projects. In particular:
- ~/projects/safe-chains — reference for code quality, TOML data
  definitions, clippy/deny config, CI patterns, CLAUDE.md conventions
- ~/projects/jjpr — reference for trait-based VCS abstraction, test
  tier structure (unit/integration/e2e), stub implementations
- ~/projects/branchdiff — reference for ratatui TUI architecture,
  notify filesystem watching, debounced event loop

Read these projects' code to match their quality bar and patterns.

## Commit style

Every commit message must use a conventional-commit prefix so `git cliff` produces real release notes. `cliff.toml` has `require_conventional = true` — a non-conventional commit will **fail the release workflow** loudly, not silently drop from the changelog.

- `feat:` → Features (minor bump candidate).
- `fix:` → Bug Fixes (patch).
- `docs:` → Documentation.
- `refactor:` → Refactor.
- `test:` → Testing.
- `perf:` → Performance.
- `chore:` / `ci:` / `build:` → Miscellaneous.
- `!` suffix marks a breaking change: `feat!:`, `fix!:`. Forces a minor bump in 0.x.

Subject ≤ 70 chars. Body explains *why* and lists any breaking migration steps.

## Pre-Commit Checklist

Before every commit, verify:
1. Every code change has a corresponding test. No exceptions.
2. `cargo clippy --all-targets -- -D warnings` passes
3. `cargo test` passes
4. Version bumped in Cargo.toml (patch for fixes, minor for features)
5. `cargo check` to regenerate Cargo.lock after version bump
6. `cargo install --path . --force` run before pushing
7. All files end with a newline

## Versioning (Semver)

**PATCH bump** (0.x.Y -> 0.x.Y+1) for: bug fixes, refactoring, test additions, dependency updates, documentation.

**MINOR bump** (0.X.y -> 0.X+1.0) for: new user-facing features (frameworks, output formats, CLI flags), new parsing capabilities.

**MAJOR bump** (X.y.z -> X+1.0.0) for: breaking CLI changes, removing frameworks, output format changes.

## Testing

cargo test
cargo test -- --ignored

All tests must pass before committing.

## Linting

cargo clippy --all-targets -- -D warnings
cargo deny check

Must pass with no warnings before committing.

`cargo deny check` runs the full suite (advisories, bans, licenses, sources), not
just licenses. Security vulnerabilities are blockers: evaluate the fix and make
the dependency bump (`cargo update -p <crate>` for a transitive dep). Only
unmaintained/transitive advisories with no upstream fix may be ignored in
`deny.toml`, each with a justifying comment.

## Development

- Framework knowledge lives in frameworks/*.toml files. See
  frameworks/SAMPLE.toml for the complete field reference.
- The registry engine in src/parse/registry.rs interprets TOML
  definitions generically. Add frameworks by adding TOML files.
- Custom handlers (Rust functions) are a last resort for behavior
  that can't be expressed in TOML. Most frameworks don't need them.
- Core types: SpecNode/SpecTree in src/parse/mod.rs,
  DiffNode/DiffTree in src/diff/types.rs
- Parsing engine in src/parse/engine.rs walks tree-sitter ASTs
  using patterns from TOML definitions
- VCS backends in src/vcs/ implement the Vcs trait
- Rename detection in src/diff/rename.rs
- Unit tests use StubVcs (in-memory file maps) and pre-built
  SharedExampleRegistry instances
- Do not add comments to code
- All files must end with a newline
- tests/file_length.rs caps every file under src/ at 400 production lines
  (inline test items are not counted). Files over it when the gate went in
  are pinned at that size and may only shrink; lower the pin when one does.
  New code goes in a new module, never into a pinned file.

## Fuzzing

Method: the `rust-fuzzing` skill. `fuzz/` is a standalone cargo-fuzz workspace
(nightly); the main crate never sees it.

The input specdiff does not control is the source of every test file on the
branch, in eight tree-sitter grammars, and the paths those files live at. Both
targets feed it through the public API, so there is no `cfg(fuzzing)` shim.

| Target | Input | Asserts |
|---|---|---|
| `outline` | `<path>\n<source>` | For every framework the path selects (plus frameworks that `extends` one, since `rust_proptest`/`rust_rstest` declare no files): outlining never panics or hangs; outlining the same source twice gives the same tree; an outline diffed against itself has no changes; renaming one node to `"<name> renamed"` diffs as exactly one rename and no add or remove. |
| `outline_diff` | `<path>\0<base>\0<head>` | The whole pipeline (`diff_files` over an in-memory `FileSource`: shared-example scan, parse, diff, path normalization, then every output format and `--filter`) never panics; identical base and head report nothing; the diff accounts for every node of both outlines exactly once (base side in order, head side as a multiset) and its kinds are coherent. |

The rename promise has two stated exceptions, skipped in the target: a name
with no whitespace-separated words (similarity is 0 against anything), and a
node whose name and kind are shared by a sibling (either copy can be paired).
Empty groups are not renamed: they need 0.7 name similarity, not 0.5.

- Seeds: `fuzz/make-seeds.sh` builds `fuzz/corpus/*/seed-*` from
  `~/projects/specdiff-tests/fixtures` and `fuzz/seed-src/` (snippets for JUnit,
  PHPUnit, Pest, pytest inheritance, rstest/proptest, which the fixtures lack).
  Re-run it after adding a fixture or framework, and commit the seeds.
- Dictionary: the same script derives `fuzz/dict/*.dict` from every one-word
  string in `frameworks/*.toml`, plus grammar punctuation and the `\0`
  separator. It regenerates, never hand-edit it.
- Local burst: `cargo +nightly fuzz build` then
  `fuzz/burst.sh fuzz/target/aarch64-apple-darwin/release/<target> <target> 60`.
  Run targets one at a time: in parallel they produce contention slow-units.
  An `outline` input costs ~15 ms under ASan (several frameworks, each parsed
  twice), so ~65-150 exec/s locally is normal.

Not fuzzed, and why:
- Framework TOMLs: compiled in by `build.rs` (`include_str!`), not read at
  runtime, so every input there is ours. `all_framework_tomls_deserialize`
  covers them.
- VCS output: git is read through git2, and jj output is parsed only as
  `commit_id` and `jj file list` lines (trimmed, non-empty), both through
  `vcs-runner`, which is its own crate.
- CLI arguments: supplied by the person running it. `--filter` is exercised
  by `outline_diff` anyway.

## Terminal UI testing

Rule: the "Terminal UIs" quality ratchet the owner keeps for every project; method: the
`tui-testing` skill. specdiff was the first project to adopt it (2026-09-28).
Never check the TUI by driving it by hand.

- Decide, then do. `src/tui/input.rs::action_for` maps a crossterm event to an
  `Action` (pure; a resize maps to `Repaint`). `src/tui/state.rs::AppState::apply`
  reduces it and returns an `Effect` (`Quit`, `Redraw`, `Repaint`, `None`). The
  run loop in `src/tui/mod.rs` only performs it. Watcher filtering is
  `src/tui/watch.rs::is_relevant_batch`. All unit-tested.
- Render. `src/tui/render.rs` tests draw into ratatui's `TestBackend` through a
  `screen(diffs, scroll, opts, w, h)` helper and assert named rows of every view:
  outline, empty, changed-only, scrolled.
- Real binary. `tests/tui.rs` on the harness `tests/support/pty.rs`
  (portable-pty + vt100): `env!("CARGO_BIN_EXE_specdiff")`, `env_clear()`, HOME
  and spec trees in a TempDir, PATH = a stub directory (one stub per program in
  `tests/tui.toml`, each recording its argv) plus `/usr/bin:/bin` for the stubs'
  own `mv` and `cat`, a fixed 30x100 terminal, polls with a 10 s deadline (short
  enough that a failing wait ends inside cargo-mutants' timeout), the child
  killed on drop. Covered: q, Esc and Ctrl-C quit and leave the alternate screen;
  a resize repaints at the new size; `--print` is coloured on a terminal and
  plain in a pipe; without `--print`, a pipe gets the printed outline; and VCS
  mode in a TempDir git repo (built by the test with real git, a local identity
  and the fake HOME) draws the branch's outline. Every test asserts no stubbed
  program ran. About 0.1 s for the suite on a quiet machine; runs in CI with
  `cargo test`.
- Not supported, so not tested: repaint after the terminal wiped its screen.
  specdiff asks for no focus events and has no redraw key; it repaints on the
  next resize, key or file change.
- No key launches anything, so `launches = []`. Programs `src/` can run: `jj`
  (through vcs-runner) and, for safety, `git` (read through git2, never run).
- `tests/tui_rules.rs` enforces the rule against `tests/tui.toml` and tests
  itself on `tests/fixtures/tui_rules/{bad,good}`. `owed` is empty; keep it so.
  A new key, view or launch gets its test and its manifest entry in the same
  change.

Found while adopting it: a resize left a stale frame until the next key or file
event, and running without `--print` into a pipe panicked in ratatui (exit 101).
Both fixed in 0.21.3.

## Differential testing

`tests/differential/` checks specdiff's outline against each framework's own
view of which tests exist. The framework is the reference: "correct" means what
it lists. Every test there is `#[ignore]`d because it runs a foreign toolchain:

    cargo test --test differential -- --ignored --nocapture

| Family | Reference (never runs a test) | Compared at |
|---|---|---|
| rust (builtin, rstest, proptest) | `cargo test --no-run --all-features` for the harnesses, then `<exe> --list --format terse` | file (walked from the target root through `a.rs`/`a/mod.rs`), module groups, test; rstest `case_N` collapse to one test with a count |
| go | `go test -json -list '.*' ./...`, `go list` for package directories | package directory and top-level function: `-list` never names subtests |
| minitest | `minitest_list.rb`: load `test/**/*_test.rb` and `test/**/test_*.rb`, print each runnable class's `runnable_methods` | file, class name split on `::`, test; a spec's `test_0001_` counter stripped |

Not compared yet: pytest (`pytest --collect-only -q`), rspec
(`rspec --dry-run --format json`), jest (`--listTests` names files only; use
`jest --json` with `--testNamePattern` that matches nothing, or vitest's
`list`), junit (needs the JUnit console launcher), phpunit/pest
(`--list-tests`, from a project's own `vendor/bin`), exunit (`mix test
--dry-run`, Elixir 1.19+, inside a mix project). Each is a `reference` function
in a new file and one arm in `main.rs`.

- The abstraction (`model.rs`): a multiset of identifiers `file > group > …
  > test [N cases]`, names put through the framework's `normalization` by a
  second implementation in `model.rs`, not the engine's, so a disagreement
  between the two shows up. A parametrised test is one identifier with a count.
- Validation of the reference, in each family: Rust asserts `--list` contains
  every `--ignored` test and builds with `--all-features`; doctests are out on
  purpose (specdiff does not outline them). Go lists every module (a nested
  `go.mod` is outside `./...`, which first hid a real project's tests) and
  asserts every package answered. `go test -list` is not quite non-executing:
  it runs `TestMain`, so a package whose `TestMain` fails makes the project a
  skip.
- `tests/differential/known.toml` lists each accepted disagreement class with
  its reason. A difference no entry explains fails; so does a class whose
  entries explained nothing in a corpus run (checked per class, not per entry,
  so an entry for a real project's file can share a corpus class). Fix a false
  positive once, in the normalisation or the list, never in the comparison.
- The report buckets differences by family, side (missing in specdiff / extra
  in specdiff) and class, and prints each unexplained one.
- Corpus: `tests/fixtures/differential/<family>/`, a small project per family
  built to hold one of each shape. Add a shape there before relying on it.
- Real projects: `SPECDIFF_DIFFERENTIAL_PROJECTS=rust=/path,go=/path` with the
  `real_projects_agree_with_their_frameworks` test. No test runs, but listing
  is not inert: cargo runs build scripts and proc macros, `go test -list` runs
  `TestMain`, and `minitest_list.rb` requires every test file (so its
  `test_helper` too). Only point it at projects you trust;
  Rust build output goes to `CARGO_TARGET_TMPDIR`.
- A missing toolchain is a loud skip (`differential: SKIPPED …` on stderr).
  `SPECDIFF_DIFFERENTIAL_REQUIRE_ALL=1` turns a skip into a failure; CI (the
  `differential` job) sets it, with Go and minitest pinned.

Found on the first run (2026-09-28), fixed with a failing test first:
`#[case::name(…)]` rstest cases were not counted (an attribute matched only a
bare identifier); Go's `required_param_type` was read by no code, so
`TestMain(m *testing.M)` was outlined as a test; minitest grouped only classes
named `Test*`, so the Rails convention `UserTest` came out flat; and a minitest
class inside a namespace `module` vanished, because minitest's inert-container
rule skipped the whole module. Recorded as decisions in TODO.md: `proptest!`
tests are never outlined (the `property_based` config is dead too), plain
modules inside a test module are flattened, rstest `#[values]` matrices.

## Mutation testing

Method: the `rust-mutation-testing` skill. Config: `.cargo/mutants.toml`. CI:
`.github/workflows/mutants.yml`, which gates nothing.

- Per change: `--in-diff` over the lines a push to main or a PR touched. Over 80
  selected mutants (a reformat, a mass rename) it skips with a warning.
- Rotating slice: each push to main also runs `--shard k/12`, k being the run
  number mod 12, so the whole tree comes round every 12 or so pushes. No
  whole-tree sweep, locally or in CI.
- The jj backend's tests are `#[ignore]`d (they need the jj CLI), so the config
  passes `--include-ignored` and every mutation run needs `jj` on PATH; CI
  installs a pinned one. Without it the baseline fails (exit 4).
- Tests that read `../specdiff-tests/fixtures` skip under cargo-mutants, which
  builds a copy of the tree elsewhere, and in CI. They cannot catch a mutant.

Adoption, 2026-09-27, cargo-mutants 27.1.0, 901 mutants in the tree:
- Slice `0/8` (113 mutants: `src/diff/mod.rs`, `src/main.rs`, part of
  `src/output/mod.rs`) took 18 minutes locally at `-j2` with other builds
  running: 175 s cold baseline build, then about 8 s a mutant. 81 caught, 19
  missed, 13 unviable: 81% (caught / (caught + missed)).
- Every miss but one now has a test: the rename-scoring ratios, budget charge
  and same-kind guard in `diff/mod.rs`, the test-file filter in VCS mode
  (`tests/cli.rs`), and the color decision, which was only observable on a
  terminal and moved to `Cli::tree_options` and `output::color_enabled`. The
  one left, in `name_similarity`, is equivalent and excluded (below).
- Slice `0/16` afterwards, with the machine quieter: 57 mutants in 2 min 45 s
  (24 s baseline), 53 caught, 4 unviable, none missed.
- N = 12 (75 mutants a slice) sits between those two: about 12 minutes at the
  contended rate, 4 at the quiet one. CI has not been timed yet; if a slice
  runs past about 15 minutes there, raise N.
- Also run whole: `src/output/truncate.rs` (38 mutants, 4 min: 28 caught, 4
  missed, 6 timeouts) and `src/vcs/jj.rs` (49 mutants, 4 min: 31 missed of 46
  viable before its new tests). Both burned down.
- Timeouts in `truncate_unchanged_runs` are real detections: mutating its loop
  counters makes it spin forever.

First CI slice (run 36363441688, 11.2 minutes, so N = 12 holds): 19 missed, in
`src/output/mod.rs` (the stats header's `> 0` guards and `push_stat_str`, the
rename line's `(was …)`, child indentation, the compact-output skip) and the
`name_is_dynamic` serialization guard in `src/parse/mod.rs`. Seventeen now
have tests asserting exact header, tree and compact text. Two are equivalent
and excluded: `modified < 0` in `push_stats_header`, because `Stats.modified`
is never counted (a real gap, recorded in TODO.md), and `==` to `!=` in
`collect_compact_lines`, whose skip is only a shortcut.

Second CI slice (run 36365534482, 9.7 minutes: 75 tested, 51 caught, 15
missed, 9 unviable), all in `src/pipeline.rs`: the shared-example registry
had almost no tests. Thirteen now have them, through an in-memory
`FileSource`: the default `list_shared_files*` are empty, the registry holds
exactly the rspec shared examples and Python base classes it should (plus a
proptest that every shared example defined in a spec file is registered),
`--framework` limits both the registry and which files are diffed or trigger
a scan, and shared files outside the change are read through the source. Two
are equivalent and excluded: the serial/parallel threshold in
`diff_with_registries`, and `&&` to `||` on `scan_spec_files_for_definitions`,
which only matters for jest, whose shared helpers never register (a real
gap, recorded in TODO.md).

Third CI slice (run 36367316465): 21 missed, in `src/tui/mod.rs` (merge-base
cache and staleness, the VCS test-file filter, watch paths, the watcher's event
filter, the tick's merge-base check, the quit and Ctrl-C bindings, the
changed-only toggle) and `src/vcs/mod.rs` (the default `files_at_revision`,
`StubVcs`). The TUI had no key, render or terminal tests at all; resolving
them was adopting the Terminal UIs rule (section above). Every one now has a
test, and a follow-up `cargo mutants` over `src/tui/` found the watch mode's
registry path, VCS mode, the rest of `StubVcs` and the header's rename guard
untested too; those have tests as well. No exclusions. Confirmed afterwards:
`src/tui/mod.rs` + `src/vcs/mod.rs` 53 mutants, 44 caught, 9 unviable, 0
missed (5 min); `src/tui/{input,state,watch,render}.rs` 76 mutants, 0 missed
after the header test (4 min).

Focused pass on `src/parse/engine.rs` (2026-09-28, owner-approved, one time), to
drain the file's backlog so rotating slices stop going red there. Before: 350
mutants in 22 minutes at `-j2` on a loaded laptop, 259 caught, 53 missed, 38
unviable: 83%. Every miss but one was a gap or unreachable code.
- Tests (module `mutation_tests` at the end of `engine.rs`, not counted by the
  ratchet) assert parse output of real snippets: marker spec, group and subtest
  line numbers for Rust, Go, pytest and JUnit (plus a proptest over leading
  blank lines for RSpec); attributes and `#[case]` counts seen through comments
  but not past the previous item; only `#[cfg(test)]` marking a test module; Go
  table-driven detection needing literal cases and a loop that runs `t.Run`;
  empty `parametrize`, `it.each` and `->with` lists not counting as
  parameterization; the camel `Test` prefix; inheritance only through
  `include`/`extend` (minitest) and the base class, not interfaces (phpunit);
  every in-scope constant substituted with the nearest winning; lowercase
  locals, symbols, hashes and nested arrays as loop receivers; a disabled loop
  expansion.
- Removed rather than excluded, because no input could reach it: loop elements'
  copy of `literal_atom_text` (now shared), `extract_name`'s `Some("constant")`
  arm (same as its fallback), a redundant clause in `has_attribute`, the
  `count > 0` check every counter already guaranteed, the length test in the
  camel-prefix strip, and the `block`/`body_statement` fallbacks after
  `child_by_field_name("body")`, which every shipped grammar provides. The file
  went from 1393 to 1351 lines.
- Real bug: JUnit `@Test(timeout = 100)` and `@Test(expected = X.class)`
  methods were dropped from the outline, because `has_annotation` compared the
  whole annotation text. Fixed in 0.21.4.
- After: 330 mutants in 21 minutes, 290 caught, 1 missed, 39 unviable. The one
  left is equivalent and excluded: `==` to `!=` on `arg_kinds` in the
  `method_call` branch of `collect_refs`, whose extra refs (strings, symbols,
  `self`) can never name a registered type. With it excluded (confirmed by
  `--list`, not a third run), 0 missed: 100%.
- That exclusion was wrong and anchored by line, so it stopped matching when
  the line moved (CI run 37100254863). Under `!=` a method-call argument
  counts as a reference too, and `include Mixins.Shared` resolves through its
  last segment to a module `Shared`. It now has a test
  (`minitest_include_of_a_method_call_is_not_a_module_reference`) and no
  exclusion.

Excluded as equivalent, with the argument next to each in `.cargo/mutants.toml`:
the two `* 1` mutants of the resume index in `truncate_unchanged_runs`
(anchored by line and column, so they reappear if the line moves), the
empty-string shortcut in `name_similarity`, the serial/parallel thresholds
in `JjVcs::files_at_revision` and `diff_with_registries`, the two
`src/output/mod.rs` mutants above, and the jest-only guard in
`build_shared_registry`.

## Adding a new framework

1. Create frameworks/<name>.toml following SAMPLE.toml patterns
2. Add tree-sitter grammar crate to Cargo.toml if new language
3. The build.rs automatically picks up new TOML files in frameworks/
4. If custom handler needed, add to src/parse/handlers/
5. Add unit tests with representative source snippets
6. Create fixture in ~/projects/specdiff-tests/fixtures/<name>/
7. Add E2E test case in tests/e2e.rs
8. Run full test suite + clippy

## Publishing

1. Bump version in Cargo.toml
2. cargo publish
3. Create git tag: jj tag v<version>
4. Push tag: jj git push --bookmark main
5. CI builds release binaries and creates GitHub Release
6. Update formula SHA in michaeldhopkins/homebrew-tap
