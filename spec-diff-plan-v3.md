# `spec-diff`: Branch-Level Test Outline Differ — v3

## Project Summary

A Rust CLI that shows how the *outline* of a test suite — its groupings and individual specs — has changed on a branch. It parses test files from two VCS revisions, normalizes them into a language-agnostic tree (including inlined shared examples), diffs the trees with rename detection, and presents the structural diff as either one-shot CLI output or a real-time TUI.

---

## 1–2. Core Concepts & Architecture

*(Unchanged from v2. See v2 for SpecTree/DiffTree types, crate layout, traits, and data flow.)*

---

## 3. Spec Suite Definitions (TOML Data Registry)

### 3.1 Design Philosophy

Following the safe-chains pattern where 420+ command behaviors are expressed as TOML data files with a generic Rust engine interpreting them, **spec-diff defines test framework knowledge as TOML data rather than scattering it across Rust match arms.** Each framework's TOML file declares everything the parser needs: how to identify test files, what AST node types constitute groups and specs, how nesting works, how parameterization is expressed, and where shared examples live.

A `SAMPLE.toml` documents every supported field. The Rust parsing engine is generic — it reads the TOML, constructs tree-sitter queries from it, and walks the resulting AST to build the SpecTree. Custom handlers (Rust functions) are a last resort for behavior that can't be expressed declaratively.

### 3.2 SAMPLE.toml — Complete Field Reference

```toml
# SAMPLE.toml — Reference for all supported spec suite definition fields.
#
# This file is not loaded by spec-diff. It documents every field the
# registry understands, when to use each one, and how they compose.
# Copy the pattern that matches your framework and fill in the data.
#
# CORE PRINCIPLE: This is a recognition engine. Only what you list is
# recognized. Unlisted AST patterns are ignored, not errored.
#
# CHOOSING THE RIGHT PATTERN:
#
# Does the framework use function-level markers (#[test], def test_)?
#   → Use "marker-based" spec detection
#
# Does the framework use DSL blocks (describe/it, test("name"))?
#   → Use "dsl-based" spec detection
#
# Does the framework use both (Go's func Test + t.Run)?
#   → Use "hybrid" with marker + nested DSL
#
# Does the framework have shared example definitions?
#   → Add a [shared] section
#
# Does the framework support parameterized tests?
#   → Add a [[parameterized]] section
#
# Does the framework support property-based testing?
#   → Add a [[property_based]] section


# ─────────────────────────────────────────────────────────────────────
# FRAMEWORK IDENTITY
# ─────────────────────────────────────────────────────────────────────

[[framework]]
name = "rspec"
language = "ruby"                    # tree-sitter grammar to use
tree_sitter_crate = "tree-sitter-ruby"

# Human-readable description for --verbose output
description = "RSpec — BDD testing for Ruby"


# ─────────────────────────────────────────────────────────────────────
# FILE DETECTION — which files contain tests for this framework
# ─────────────────────────────────────────────────────────────────────
#
# A file must match at least one pattern to be considered.
# Patterns are checked against the path relative to the project root.
# Glob syntax: * matches any filename chars, ** matches directories.

[framework.files]
extensions = [".rb"]
include_globs = [
    "spec/**/*_spec.rb",
    "spec/**/*_test.rb",
]
exclude_globs = [
    "spec/spec_helper.rb",
    "spec/rails_helper.rb",
]

# Some frameworks require AST confirmation — the file must contain
# matching constructs even if the filename matches. Set to true for
# frameworks where test files don't follow naming conventions (e.g.,
# Rust inline #[cfg(test)] modules in any .rs file).
require_ast_confirmation = false


# ─────────────────────────────────────────────────────────────────────
# GROUP NODES — AST patterns that define nesting/grouping
# ─────────────────────────────────────────────────────────────────────
#
# Each [[framework.group]] defines one type of group construct.
# Groups can nest (describe inside describe, context inside describe).
# The "name_source" field tells the parser where to extract the
# group's human-readable name from the AST.

[[framework.group]]
# RSpec describe blocks
ast_type = "call"                      # tree-sitter node type
method_names = ["describe", "context", "feature"]
name_source = "first_argument"         # extract name from first arg
name_source_type = "string_literal"    # expect a string literal
# What the name_source can be:
#   "first_argument"    — first positional arg (describe "User")
#   "first_argument"    — with name_source_type = "constant" for
#                         describe(User) where User is a class
#   "identifier"        — the function/module name itself
#   "attribute_value"   — value from a decorator/attribute

[[framework.group]]
# RSpec scenario blocks (Capybara)
ast_type = "call"
method_names = ["scenario"]
name_source = "first_argument"
name_source_type = "string_literal"


# ─────────────────────────────────────────────────────────────────────
# SPEC NODES — AST patterns that define individual test cases
# ─────────────────────────────────────────────────────────────────────

[[framework.spec]]
ast_type = "call"
method_names = ["it", "specify", "example"]
name_source = "first_argument"
name_source_type = "string_literal"

# Optional: some specs have no description string (bare `it { ... }`)
# When allow_anonymous is true, the parser generates a name from
# the block's first assertion or "anonymous spec at line N".
allow_anonymous = true


# ─────────────────────────────────────────────────────────────────────
# SPEC NAME NORMALIZATION
# ─────────────────────────────────────────────────────────────────────
#
# Controls how raw spec names are cleaned up for the outline.

[framework.normalization]
# Strip these prefixes from function names before display
strip_prefixes = ["test_"]
# Replace underscores with spaces in function names
underscore_to_space = true
# Strip "test" prefix from CamelCase names (TestValidateEmail → ValidateEmail)
strip_camel_test_prefix = true
# Keep raw names as-is (overrides all above)
raw = false


# ─────────────────────────────────────────────────────────────────────
# FILE PATH GROUPING
# ─────────────────────────────────────────────────────────────────────
#
# Controls how the file path becomes the top-level group name.

[framework.path_grouping]
# Directories to strip from the beginning of the path
strip_prefixes = ["spec/", "test/", "tests/"]
# File suffixes to strip (before extension)
strip_suffixes = ["_spec", "_test"]
# Separator for path components in the group name
separator = "::"
# Strip the file extension
strip_extension = true
# Example: spec/models/user_spec.rb → models::user


# ─────────────────────────────────────────────────────────────────────
# PARAMETERIZED TESTS
# ─────────────────────────────────────────────────────────────────────
#
# Each [[framework.parameterized]] describes one way the framework
# supports parameterization. A single framework can have multiple
# parameterization patterns (e.g., pytest has @parametrize and
# fixture params).

# (RSpec does not have built-in parameterization, so this section
# would be absent. See pytest.toml and rust.toml for examples.)

# [[framework.parameterized]]
# # How the parameterization is expressed in the AST
# detection = "decorator"              # decorator | attribute | macro | method_call
# decorator_name = "pytest.mark.parametrize"
#
# # Where to find the case count
# case_count_source = "second_argument" # count items in the list
# case_count_type = "list_length"       # list_length | attribute_count
#
# # Where to find human-readable case labels (optional)
# label_source = "ids_kwarg"            # ids= keyword argument
# label_fallback = "first_element"      # fall back to first element of each tuple


# ─────────────────────────────────────────────────────────────────────
# PROPERTY-BASED TESTS
# ─────────────────────────────────────────────────────────────────────

# [[framework.property_based]]
# detection = "macro"
# macro_name = "proptest"
# # or:
# detection = "decorator"
# decorator_name = "given"             # Hypothesis @given decorator
# decorator_module = "hypothesis"


# ─────────────────────────────────────────────────────────────────────
# SHARED EXAMPLES — definitions and inclusion sites
# ─────────────────────────────────────────────────────────────────────
#
# Shared examples are test fragments defined in one place and
# included in another. The registry needs to know:
# 1. How to find definition files
# 2. How definitions are expressed in the AST
# 3. How inclusions are expressed in the AST
# 4. Whether inclusion creates a nested group or inlines directly

[framework.shared]
# Where definition files typically live (globs relative to project root)
definition_globs = [
    "spec/support/**/*.rb",
    "spec/shared_examples/**/*.rb",
    "spec/shared_contexts/**/*.rb",
    # Definitions can also appear inline in any spec file
]
# Should we also scan spec files themselves for inline definitions?
scan_spec_files_for_definitions = true

[[framework.shared.definition]]
# shared_examples "name" do ... end
ast_type = "call"
method_names = ["shared_examples", "shared_examples_for", "shared_context"]
name_source = "first_argument"
name_source_type = "string_literal"
# The block body contains group/spec nodes parsed recursively
body = "block"

[[framework.shared.inclusion]]
# include_examples "name"
ast_type = "call"
method_names = ["include_examples", "include_context"]
name_source = "first_argument"
name_source_type = "string_literal"
# Inlines directly into the current group (no nesting)
nesting = "inline"

[[framework.shared.inclusion]]
# it_behaves_like "name"
ast_type = "call"
method_names = ["it_behaves_like", "it_should_behave_like"]
name_source = "first_argument"
name_source_type = "string_literal"
# Creates a nested group named "behaves like <name>"
nesting = "nested"
nested_name_template = "behaves like {name}"


# ─────────────────────────────────────────────────────────────────────
# MARKER-BASED DETECTION — for attribute/decorator-driven frameworks
# ─────────────────────────────────────────────────────────────────────
#
# Used instead of (or alongside) DSL-based detection when test
# identity comes from attributes/decorators rather than method calls.

# [[framework.marker]]
# # Rust #[test] attribute
# marker_type = "attribute"             # attribute | decorator | pragma
# marker_name = "test"
# applies_to = "function"               # function | method | class
# creates = "spec"                      # spec | group
#
# [[framework.marker]]
# # Rust #[cfg(test)] on modules
# marker_type = "attribute"
# marker_name = "cfg"
# marker_argument = "test"
# applies_to = "module"
# creates = "group"
#
# [[framework.marker]]
# # Python class-based: class names starting with "Test"
# marker_type = "name_pattern"
# pattern = "^Test"
# applies_to = "class"
# creates = "group"
#
# [[framework.marker]]
# # Python function-based: function names starting with "test_"
# marker_type = "name_pattern"
# pattern = "^test_"
# applies_to = "function"
# creates = "spec"


# ─────────────────────────────────────────────────────────────────────
# HYBRID DETECTION — nested test discovery within markers
# ─────────────────────────────────────────────────────────────────────
#
# For frameworks where a marker defines the test entry point, but
# subtests are discovered via method calls inside the function body.

# [[framework.nested_discovery]]
# # Go t.Run("name", func(t) { ... })
# parent = "marker"                     # look inside marker-detected functions
# ast_type = "call"
# receiver = "t"                        # or "b" for benchmarks
# method_name = "Run"
# name_source = "first_argument"
# name_source_type = "string_literal"
# creates = "spec"                      # or "group" if it contains its own t.Run


# ─────────────────────────────────────────────────────────────────────
# TABLE-DRIVEN TEST DETECTION — heuristic for Go-style patterns
# ─────────────────────────────────────────────────────────────────────
#
# Detects the pattern: slice literal → range loop → t.Run
# Counts composite literals in the slice to get case count.

# [framework.table_driven]
# enabled = true
# slice_pattern = "composite_literal"   # tree-sitter node type for struct literals
# loop_pattern = "for_range"            # tree-sitter node type for range loop
# run_call = "t.Run"                    # the subtest call inside the loop


# ─────────────────────────────────────────────────────────────────────
# CUSTOM HANDLER — requires Rust code for validation
# ─────────────────────────────────────────────────────────────────────
#
# Last resort when the framework's test patterns can't be expressed
# declaratively. The handler name references a Rust function.
#
# Before reaching for this: can you express the detection as markers,
# DSL groups/specs, or nested discovery? Most frameworks can.

# handler = "jest_dynamic"  # references Rust handler by name
```

### 3.3 Concrete Framework Definitions

#### `frameworks/rspec.toml`

```toml
[[framework]]
name = "rspec"
language = "ruby"
tree_sitter_crate = "tree-sitter-ruby"
description = "RSpec — BDD testing for Ruby"

[framework.files]
extensions = [".rb"]
include_globs = ["spec/**/*_spec.rb"]
exclude_globs = ["spec/spec_helper.rb", "spec/rails_helper.rb"]
require_ast_confirmation = false

[[framework.group]]
ast_type = "call"
method_names = ["describe", "context", "feature"]
name_source = "first_argument"
name_source_type = "string_literal"

[[framework.group]]
ast_type = "call"
method_names = ["describe"]
name_source = "first_argument"
name_source_type = "constant"

[[framework.spec]]
ast_type = "call"
method_names = ["it", "specify", "example"]
name_source = "first_argument"
name_source_type = "string_literal"
allow_anonymous = true

[[framework.spec]]
ast_type = "call"
method_names = ["scenario"]
name_source = "first_argument"
name_source_type = "string_literal"

[framework.normalization]
raw = true

[framework.path_grouping]
strip_prefixes = ["spec/"]
strip_suffixes = ["_spec"]
separator = "::"
strip_extension = true

[framework.shared]
definition_globs = [
    "spec/support/**/*.rb",
    "spec/shared_examples/**/*.rb",
    "spec/shared_contexts/**/*.rb",
]
scan_spec_files_for_definitions = true

[[framework.shared.definition]]
ast_type = "call"
method_names = ["shared_examples", "shared_examples_for", "shared_context"]
name_source = "first_argument"
name_source_type = "string_literal"
body = "block"

[[framework.shared.inclusion]]
ast_type = "call"
method_names = ["include_examples", "include_context"]
name_source = "first_argument"
name_source_type = "string_literal"
nesting = "inline"

[[framework.shared.inclusion]]
ast_type = "call"
method_names = ["it_behaves_like", "it_should_behave_like"]
name_source = "first_argument"
name_source_type = "string_literal"
nesting = "nested"
nested_name_template = "behaves like {name}"
```

#### `frameworks/rust_builtin.toml`

```toml
[[framework]]
name = "rust_builtin"
language = "rust"
tree_sitter_crate = "tree-sitter-rust"
description = "Rust built-in #[test] + #[cfg(test)]"

[framework.files]
extensions = [".rs"]
include_globs = ["src/**/*.rs", "tests/**/*.rs"]
require_ast_confirmation = true

[[framework.marker]]
marker_type = "attribute"
marker_name = "test"
applies_to = "function"
creates = "spec"

[[framework.marker]]
marker_type = "attribute"
marker_name = "cfg"
marker_argument = "test"
applies_to = "module"
creates = "group"

[framework.normalization]
strip_prefixes = ["test_"]
underscore_to_space = true

[framework.path_grouping]
strip_prefixes = ["src/", "tests/"]
strip_suffixes = []
separator = "::"
strip_extension = true
```

#### `frameworks/rust_rstest.toml`

```toml
[[framework]]
name = "rust_rstest"
language = "rust"
tree_sitter_crate = "tree-sitter-rust"
description = "rstest — parameterized testing for Rust"

# Extends rust_builtin — both can be active on the same file.
# rstest specs are detected by the #[rstest] attribute instead of #[test].
extends = "rust_builtin"

[[framework.marker]]
marker_type = "attribute"
marker_name = "rstest"
applies_to = "function"
creates = "spec"

[[framework.parameterized]]
detection = "attribute"
attribute_name = "case"
case_count_type = "attribute_count"
label_source = "first_literal_in_attribute"
```

#### `frameworks/rust_proptest.toml`

```toml
[[framework]]
name = "rust_proptest"
language = "rust"
tree_sitter_crate = "tree-sitter-rust"
description = "proptest — property-based testing for Rust"
extends = "rust_builtin"

[[framework.property_based]]
detection = "macro"
macro_name = "proptest"
```

#### `frameworks/pytest.toml`

```toml
[[framework]]
name = "pytest"
language = "python"
tree_sitter_crate = "tree-sitter-python"
description = "pytest — Python testing framework"

[framework.files]
extensions = [".py"]
include_globs = [
    "tests/**/test_*.py",
    "tests/**/*_test.py",
    "test_*.py",
    "*_test.py",
]
require_ast_confirmation = false

[[framework.marker]]
marker_type = "name_pattern"
pattern = "^Test"
applies_to = "class"
creates = "group"

[[framework.marker]]
marker_type = "name_pattern"
pattern = "^test_"
applies_to = "function"
creates = "spec"

[[framework.parameterized]]
detection = "decorator"
decorator_name = "pytest.mark.parametrize"
case_count_source = "second_argument"
case_count_type = "list_length"
label_source = "ids_kwarg"
label_fallback = "first_element"

[[framework.property_based]]
detection = "decorator"
decorator_name = "given"
decorator_module = "hypothesis"

[framework.normalization]
strip_prefixes = ["test_"]
underscore_to_space = true
strip_camel_test_prefix = true

[framework.path_grouping]
strip_prefixes = ["tests/", "test/"]
strip_suffixes = ["_test"]
separator = "::"
strip_extension = true

[framework.shared]
definition_globs = ["**/conftest.py"]
scan_spec_files_for_definitions = false

[[framework.shared.definition]]
# pytest conftest fixtures aren't specs. But test base classes are.
# Detect classes with test methods that are subclassed by test files.
ast_type = "class_definition"
method_names = []
name_source = "identifier"
body = "class_body"
detection_strategy = "inheritance"
# The parser detects when a test class inherits from a class that
# has test methods, and inlines the parent's test methods.
handler = "pytest_inheritance"
```

#### `frameworks/jest.toml`

```toml
[[framework]]
name = "jest"
language = "javascript"
tree_sitter_crate = "tree-sitter-javascript"
description = "Jest / Vitest / Mocha — JavaScript testing"

# Also handles TypeScript via tree-sitter-typescript
additional_languages = ["typescript"]
additional_crates = ["tree-sitter-typescript"]

[framework.files]
extensions = [".js", ".jsx", ".ts", ".tsx"]
include_globs = [
    "**/*.test.js", "**/*.test.ts", "**/*.test.jsx", "**/*.test.tsx",
    "**/*.spec.js", "**/*.spec.ts", "**/*.spec.jsx", "**/*.spec.tsx",
    "**/__tests__/**/*.js", "**/__tests__/**/*.ts",
]
require_ast_confirmation = false

[[framework.group]]
ast_type = "call_expression"
method_names = ["describe"]
name_source = "first_argument"
name_source_type = "string_literal"

[[framework.spec]]
ast_type = "call_expression"
method_names = ["it", "test"]
name_source = "first_argument"
name_source_type = "string_literal"

[[framework.parameterized]]
detection = "method_call"
method_names = ["describe.each", "it.each", "test.each"]
case_count_source = "first_argument"
case_count_type = "array_length"

[framework.normalization]
raw = true

[framework.path_grouping]
strip_prefixes = ["__tests__/", "test/", "tests/"]
strip_suffixes = [".test", ".spec"]
separator = "::"
strip_extension = true

[framework.shared]
definition_globs = [
    "**/__tests__/helpers/**/*.js",
    "**/__tests__/helpers/**/*.ts",
    "**/test/helpers/**/*.js",
    "**/test/helpers/**/*.ts",
]
scan_spec_files_for_definitions = false

[[framework.shared.definition]]
# Functions that contain describe/it calls and are exported
ast_type = "function_declaration"
name_source = "identifier"
body = "function_body"
detection_strategy = "contains_specs_and_exported"
handler = "jest_shared_helpers"
```

#### `frameworks/go.toml`

```toml
[[framework]]
name = "go_testing"
language = "go"
tree_sitter_crate = "tree-sitter-go"
description = "Go testing package — func Test + t.Run"

[framework.files]
extensions = [".go"]
include_globs = ["**/*_test.go"]
require_ast_confirmation = false

[[framework.marker]]
marker_type = "name_pattern"
pattern = "^Test"
applies_to = "function"
creates = "spec"
# Go test functions must accept *testing.T
required_param_type = "*testing.T"

[[framework.nested_discovery]]
parent = "marker"
ast_type = "call_expression"
receiver = "t"
method_name = "Run"
name_source = "first_argument"
name_source_type = "string_literal"
creates = "spec"

[framework.table_driven]
enabled = true
slice_pattern = "composite_literal"
loop_pattern = "for_statement"
run_call = "t.Run"

[framework.normalization]
strip_camel_test_prefix = true
underscore_to_space = false

[framework.path_grouping]
strip_prefixes = []
strip_suffixes = ["_test"]
separator = "::"
strip_extension = true
```

#### `frameworks/elixir_exunit.toml`

```toml
[[framework]]
name = "exunit"
language = "elixir"
tree_sitter_crate = "tree-sitter-elixir"
description = "ExUnit — Elixir test framework"

[framework.files]
extensions = [".exs"]
include_globs = ["test/**/*_test.exs"]
require_ast_confirmation = false

[[framework.group]]
ast_type = "call"
method_names = ["describe"]
name_source = "first_argument"
name_source_type = "string_literal"

[[framework.spec]]
ast_type = "call"
method_names = ["test"]
name_source = "first_argument"
name_source_type = "string_literal"

[framework.normalization]
raw = true

[framework.path_grouping]
strip_prefixes = ["test/"]
strip_suffixes = ["_test"]
separator = "::"
strip_extension = true
```

### 3.4 Registry Engine

The registry engine lives in `src/parse/registry.rs`. It:

1. Loads all `frameworks/*.toml` files at compile time via `include_str!` (same pattern as safe-chains' `commands/*.toml`).
2. Deserializes them into strongly-typed `FrameworkDef` structs using `serde`.
3. For each file to parse, selects matching frameworks by extension and glob.
4. Constructs tree-sitter queries dynamically from the TOML definitions.
5. Walks the AST, matching against group/spec/marker/parameterized patterns.
6. Resolves shared example inclusions via the `SharedExampleRegistry`.
7. Falls back to named Rust handlers for `handler = "..."` fields.

The `FrameworkDef` struct hierarchy mirrors the TOML exactly — each field is a struct or enum. Adding a new framework means adding a TOML file; adding a new *pattern type* means adding a field to the struct and teaching the engine to interpret it.

### 3.5 Framework Detection for a File

A single file can match multiple frameworks (e.g., a Rust file might match both `rust_builtin` and `rust_rstest`). When `extends` is set, the child framework's patterns supplement the parent's. Results are merged: if rstest finds a parameterized spec that `rust_builtin` would have found as a plain spec, the parameterized version wins.

---

## 4. Performance

### 4.1 Analysis of Hot Paths

The tool's performance profile has four distinct phases:

| Phase | Typical cost | Scaling factor |
|-------|-------------|----------------|
| VCS diff (changed files list) | 5–50ms | Repo size, commit count |
| File reading at revisions | 10–200ms | Number of changed test files |
| Tree-sitter parsing | 1–5ms per file | File size, query complexity |
| Tree diff + rename detection | <1ms | Number of spec nodes |

**The bottleneck is phase 2** — reading file contents from VCS. Tree-sitter parsing is extremely fast (it's a C parser under the hood). The tree diff is trivial unless there are thousands of spec nodes.

### 4.2 VCS Read Optimization

**Git (git2):** reading file contents at a revision is a tree walk + blob lookup, all in-process with no subprocess overhead. For N changed files, this is N blob reads — fast.

**Jujutsu:** each `jj file show` is a subprocess invocation. For N files, that's 2N subprocesses (base + head). Mitigate with:
- **Batch reading:** use `jj diff --from X --to Y` to get the full diff content in one call, then extract file contents from the unified diff. This is one subprocess instead of 2N.
- **Parallel reads:** if batch isn't sufficient, use `rayon` to parallelize `jj file show` calls. Jujutsu handles concurrent reads safely.

**Shared example resolution adds reads:** when a shared definition file changed, we need to find all test files that include it (to detect propagated changes). This reverse-dependency scan uses `Vcs::files_matching` (one call) followed by a grep for inclusion method names. This is a text search, not a full parse — fast even on large projects.

### 4.3 Caching Strategy

For `--watch` mode, cache aggressively:

- **Tree-sitter parse trees:** cache the `SpecTree` for each file, keyed by file path + content hash. When a file change event fires, only re-parse that file.
- **SharedExampleRegistry:** rebuild incrementally. When a shared definition file changes, re-parse it and update the registry entry. Then re-parse only the files that include that shared example.
- **VCS state:** cache the merge-base computation. Invalidate only on branch change (detectable via HEAD ref change).
- **Unchanged files:** in `--watch` mode, the baseline SpecTree (from the merge-base revision) never changes. Cache it once.

### 4.4 Watch Mode Architecture (branchdiff Pattern)

Following branchdiff's architecture — `ratatui` TUI with `notify` filesystem watcher and two-section display:

```
┌─────────────────────────────────────────────────┐
│ spec-diff  main → feature/auth  +3 -1 →2 ~2    │
│                                                 │
│   models::User                                  │
│ +   validates uniqueness of username            │
│     validates email format                      │
│ →   requires password → validates password len… │
│ ~   associations                                │
│ +     has many comments                         │
│       has many posts                            │
│   models::Post                                  │
│     belongs to user                             │
│ + requests::admin                               │
│ +   DELETE /users/:id returns 403               │
│                                                 │
│ [q]uit  [/]filter  [c]hanged-only  arrows/jk    │
└─────────────────────────────────────────────────┘
```

**Event loop (modeled on branchdiff):**

```rust
enum AppEvent {
    Key(KeyEvent),
    FileChanged(PathBuf),
    Tick,
}
```

1. `notify::RecommendedWatcher` watches the project directory, filtered to test file extensions.
2. Events are debounced (200ms) and sent to a channel.
3. The TUI event loop selects between key events, file change events, and a tick timer.
4. On file change: re-read the changed file from disk (it's the working copy), re-parse, re-diff, re-render. The base revision's tree is cached.
5. Debouncing prevents re-diffing on every keystroke during active editing.

**Key difference from branchdiff:** branchdiff diffs raw file content; spec-diff diffs parsed spec trees. The extra parsing step is fast enough (<5ms per file) that debounced watch still feels instant.

### 4.5 Startup Time

Target: <200ms from invocation to first render for a typical project (100 changed test files). Tree-sitter grammar loading is the main fixed cost (~50ms for all grammars). TOML framework definitions are compiled in via `include_str!` and deserialized once at startup.

---

## 5–8. Shared Examples, Rename Detection, VCS, Presentation

*(Unchanged from v2. See v2 for SharedExampleRegistry, rename algorithm, VCS backends, CLI/TUI/JSON rendering.)*

---

## 9. CLI Interface

*(Unchanged from v2.)*

---

## 10. Testing Strategy

*(Unchanged from v2, plus:)*

### 10.4 Framework Definition Tests

Each `frameworks/*.toml` file has a corresponding test module that:

1. Deserializes the TOML and asserts it's valid (no unknown fields, all referenced handlers exist).
2. Feeds representative source snippets through the registry engine using that framework definition.
3. Asserts the resulting `SpecTree` matches expectations.
4. Uses `insta` snapshots for the SpecTree output.

This ensures that framework definitions are tested at the data level — changing a TOML file triggers the same test harness.

---

## 11. Code Quality Requirements

*(Unchanged from v2.)*

---

## 12. Publishing

### 12.1 crates.io

Standard `cargo publish` workflow. Ensure `Cargo.toml` has:
- `license = "MIT OR Apache-2.0"`
- `repository`, `homepage`, `documentation` URLs
- `categories` and `keywords`
- `include` list (exclude fixtures, scripts)

### 12.2 Homebrew Tap

Publish to `michaeldhopkins/homebrew-tap` (same tap that hosts safe-chains and jjpr).

**Formula file:** `Formula/spec-diff.rb`

```ruby
class SpecDiff < Formula
  desc "Show test outline changes on a branch"
  homepage "https://github.com/michaeldhopkins/spec-diff"
  url "https://github.com/michaeldhopkins/spec-diff/archive/refs/tags/v#{version}.tar.gz"
  license any_of: ["MIT", "Apache-2.0"]

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
    # Man page
    man1.install "target/release/build/spec-diff-*/out/spec-diff.1" if File.exist?("...")
    # Shell completions
    generate_completions_from_executable(bin/"spec-diff", "--completions")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/spec-diff --version")
  end
end
```

**Release workflow:** The GitHub Actions release job (triggered by a version tag) should:

1. Build release binaries for macOS (aarch64, x86_64) and Linux (x86_64, aarch64).
2. Create a GitHub Release with the binaries attached.
3. Compute the SHA256 of the source tarball.
4. Update the formula in `homebrew-tap` with the new version and SHA (can be automated with a script or a second workflow that opens a PR on the tap repo).

**Testing the formula:**

```bash
brew install --build-from-source michaeldhopkins/tap/spec-diff
brew test michaeldhopkins/tap/spec-diff
brew audit --strict michaeldhopkins/tap/spec-diff
```

### 12.3 Pre-built Binaries

Follow safe-chains' pattern: signed, notarized macOS binaries + Linux binaries attached to GitHub Releases. Provide a `curl | tar` one-liner in the README.

---

## 13. Implementation Build Order

*(Unchanged from v2, with one addition:)*

### Step 0: Framework TOML Definitions + Registry Engine

Before any parsing code, build:
1. The `FrameworkDef` struct hierarchy (mirrors TOML schema).
2. The registry engine that loads TOML definitions via `include_str!`.
3. The generic tree-sitter query builder that constructs queries from TOML patterns.
4. Unit tests: deserialize each framework TOML, assert validity.

This is the foundation everything else builds on. The parsers in Steps 3–8 are then thin — they're framework TOML files plus occasional handler functions, not bespoke parsing code.

---

## 14. CLAUDE.md (For the Coding Agent)

```markdown
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
6. Create fixture repo branch pair in spec-diff-fixtures
7. Add E2E test case in tests/e2e.rs
8. Run full test suite + clippy

## Publishing

1. Bump version in Cargo.toml
2. cargo publish
3. Create git tag: git tag v<version>
4. Push tag: git push origin v<version>
5. CI builds release binaries and creates GitHub Release
6. Update formula SHA in michaeldhopkins/homebrew-tap
```
