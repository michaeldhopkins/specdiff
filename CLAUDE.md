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
- VCS backends in src/vcs/ implement the Vcs trait
- Rename detection in src/diff/rename.rs
- Unit tests use StubVcs (in-memory file maps) and pre-built
  SharedExampleRegistry instances
- Do not add comments to code
- All files must end with a newline
- Bump version in Cargo.toml with each commit using semver

## Adding a new framework

1. Create frameworks/<name>.toml following SAMPLE.toml patterns
2. Add tree-sitter grammar crate to Cargo.toml if new language
3. Register the TOML file via include_str! in src/parse/registry.rs
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
