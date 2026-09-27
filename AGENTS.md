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

Excluded as equivalent, with the argument next to each in `.cargo/mutants.toml`:
the two `* 1` mutants of the resume index in `truncate_unchanged_runs`
(anchored by line and column, so they reappear if the line moves), the
empty-string shortcut in `name_similarity`, and the serial/parallel threshold
in `JjVcs::files_at_revision`.

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
