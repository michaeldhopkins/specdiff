# specdiff

Show how the test outline of a project has changed on a branch.

Parses test files from two VCS revisions using tree-sitter, normalizes them into a language-agnostic tree, diffs the trees with rename detection, and presents the structural diff as CLI output or a real-time TUI.

## Features

- **Multi-framework support**: RSpec, Minitest, pytest, Jest/Vitest/Mocha, Go testing, Rust #[test], ExUnit — defined declaratively in TOML
- **Git and Jujutsu (jj)** support with automatic backend detection
- Tree-sitter parsing for accurate, syntax-aware test discovery
- Shared example resolution (RSpec shared_examples, pytest conftest)
- Rename detection with similarity scoring
- Parameterized test awareness (rstest, pytest.mark.parametrize, Jest .each)
- Output formats: colored tree (default), JSON, compact
- Watch mode TUI with live refresh on file changes

## Requirements

- **Git**: Any reasonably modern git (1.7+)
- **Jujutsu** (optional): If a `.jj` directory is present, specdiff uses jj automatically

## Installation

### Homebrew (macOS/Linux)

```bash
brew tap michaeldhopkins/tap
brew install specdiff
```

### From source

```bash
cargo install --git https://github.com/michaeldhopkins/specdiff
```

### Manual download

Download binaries from [GitHub Releases](https://github.com/michaeldhopkins/specdiff/releases).

## Usage

```bash
specdiff
```

Run in a git/jj repository. Opens a TUI that shows test outline changes on the current branch and live-refreshes when test files change.

Keybindings: `q` quit, `c` toggle changed-only, `j/k` scroll, `PgUp/PgDn` page scroll.

### Non-interactive output

```bash
specdiff --print
specdiff -p --format json
specdiff -p --format compact
```

### Options

| Flag | Description |
|------|-------------|
| `-p`, `--print` | Print diff to stdout and exit (non-interactive) |
| `--base <REV>` | Base revision (default: auto-detected) |
| `--head <REV>` | Head revision (default: working copy) |
| `--format <FORMAT>` | Output format: `tree` (default), `json`, `compact` |
| `--changed-only` | Only show changed specs |
| `--framework <NAME>` | Force a specific framework |
| `--filter <PATTERN>` | Filter specs by name pattern |
| `--no-color` | Disable colored output |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

### Example output

```
models::User
+   validates uniqueness of username
    validates email format
->  requires password -> validates password length
~   associations
+     has many comments
      has many posts
+ requests::admin
+   DELETE /users/:id returns 403
```

Legend: `+` added, `-` removed, `->` renamed, `~` modified (children changed)

## Supported Frameworks

| Framework | Language | Detection |
|-----------|----------|-----------|
| RSpec | Ruby | DSL (`describe`/`it`/`context`) |
| Minitest | Ruby | Class names (`Test*`) + method names (`test_*`) |
| pytest | Python | Class names (`Test*`) + function names (`test_*`) |
| Jest / Vitest / Mocha | JavaScript/TypeScript | DSL (`describe`/`it`/`test`) |
| Go testing | Go | Function names (`Test*`) + `t.Run` subtests |
| Rust built-in | Rust | `#[test]` + `#[cfg(test)]` attributes |
| rstest | Rust | `#[rstest]` parameterized tests |
| proptest | Rust | `proptest!` macro |
| ExUnit | Elixir | DSL (`describe`/`test`) |

Framework knowledge is defined in TOML data files under `frameworks/`. Adding a new framework requires only a new TOML file in most cases.

## Contributing

Requires Rust 1.85+.

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo deny check licenses
```

## License

Dual-licensed under MIT or Apache-2.0.
