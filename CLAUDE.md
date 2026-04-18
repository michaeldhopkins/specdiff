# CLAUDE.md

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
cargo deny check licenses

Must pass with no warnings before committing.

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
