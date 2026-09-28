//! File-length gate for `src/`, adapted from cmdproof's `engine/tests/file_length.rs`.
//!
//! Inline test items are not counted, so adding tests never breaks the build. Files already over
//! the limit when the gate went in are pinned at that size: they may shrink, never grow, and a
//! shrink must lower the pin in the same change so the file cannot grow back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::visit::Visit;

const LIMIT: usize = 400;

/// Production line counts on 2026-09-26, when the gate went in. Never add to this list: a file
/// that outgrows the limit is split.
fn pinned() -> HashMap<&'static str, usize> {
    HashMap::from([
        ("src/diff/mod.rs", 405),
        ("src/parse/engine.rs", 1363),
        ("src/pipeline.rs", 403),
    ])
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("test")
            || (a.path().is_ident("cfg")
                && a.parse_args::<syn::Meta>().is_ok_and(|m| m.path().is_ident("test")))
    })
}

struct TestItems(Vec<(usize, usize)>);

impl TestItems {
    fn add(&mut self, attrs: &[syn::Attribute], item: &impl Spanned) -> bool {
        if !test_only(attrs) {
            return false;
        }
        let span = item.span();
        self.0.push((span.start().line, span.end().line));
        true
    }
}

impl<'a> Visit<'a> for TestItems {
    fn visit_item(&mut self, i: &'a syn::Item) {
        let attrs = match i {
            syn::Item::Const(x) => &x.attrs,
            syn::Item::Enum(x) => &x.attrs,
            syn::Item::Fn(x) => &x.attrs,
            syn::Item::Impl(x) => &x.attrs,
            syn::Item::Macro(x) => &x.attrs,
            syn::Item::Mod(x) => &x.attrs,
            syn::Item::Static(x) => &x.attrs,
            syn::Item::Struct(x) => &x.attrs,
            syn::Item::Trait(x) => &x.attrs,
            syn::Item::Type(x) => &x.attrs,
            syn::Item::Use(x) => &x.attrs,
            _ => return syn::visit::visit_item(self, i),
        };
        if !self.add(attrs, i) {
            syn::visit::visit_item(self, i);
        }
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        if !self.add(&f.attrs, f) {
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
}

/// Read from the parsed file, not the text: text rules in other projects were fooled by a
/// `}` in column 0 inside a fixture string and by a test module sitting between functions.
fn production_lines(source: &str) -> usize {
    let file = syn::parse_file(source).unwrap_or_else(|e| panic!("does not parse: {e}"));
    let mut tests = TestItems(Vec::new());
    tests.visit_file(&file);
    let total = source.lines().count();
    (1..=total)
        .filter(|line| !tests.0.iter().any(|(a, b)| (a..=b).contains(&line)))
        .count()
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

fn verdict(
    relative: &str,
    lines: usize,
    limit: usize,
    pinned: &HashMap<&'static str, usize>,
) -> Option<String> {
    match pinned.get(relative) {
        Some(&ceiling) if lines > ceiling => Some(format!(
            "{relative}: {lines} lines, up from its pinned {ceiling}. It is already over the {limit}-line limit; split it rather than growing it further."
        )),
        Some(_) if lines <= limit => Some(format!(
            "{relative}: down to {lines} lines, under the {limit} limit, so remove its entry from `pinned()`."
        )),
        Some(&ceiling) if lines < ceiling => Some(format!(
            "{relative}: down to {lines} lines from its pinned {ceiling}. Lower its pin to {lines} so it cannot grow back."
        )),
        Some(_) => None,
        None if lines > limit => Some(format!(
            "{relative}: {lines} lines, over the {limit} limit. Split it (inline tests are not counted, so they are not the cause)."
        )),
        None => None,
    }
}

#[test]
fn no_file_outgrows_its_limit() {
    let root = root();
    let pinned = pinned();
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    assert!(
        files.len() > 10,
        "only {} `.rs` files under `src`; the walk is broken, not the tree",
        files.len()
    );
    let mut failures = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        if let Some(f) = verdict(&relative, production_lines(&source), LIMIT, &pinned) {
            failures.push(f);
        }
    }
    failures.sort();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn the_ratchet_holds_a_pinned_file_to_its_size() {
    let pinned = HashMap::from([("a.rs", 500)]);
    assert!(verdict("a.rs", 500, 400, &pinned).is_none());
    assert!(verdict("a.rs", 450, 400, &pinned).is_some_and(|m| m.contains("Lower its pin to 450")));
    assert!(verdict("a.rs", 501, 400, &pinned).is_some_and(|m| m.contains("up from its pinned")));
    assert!(verdict("a.rs", 400, 400, &pinned).is_some_and(|m| m.contains("remove its entry")));
    assert!(verdict("b.rs", 400, 400, &pinned).is_none());
    assert!(verdict("b.rs", 401, 400, &pinned).is_some_and(|m| m.contains("over the")));
}

#[test]
fn every_pinned_file_still_exists() {
    let root = root();
    let missing: Vec<&str> = pinned()
        .keys()
        .copied()
        .filter(|r| !root.join(r).exists())
        .collect();
    assert!(missing.is_empty(), "pinned files no longer at these paths: {missing:?}");
}

#[test]
fn only_test_items_are_left_out_of_the_count() {
    let cases: &[(&str, &str, usize)] = &[
        ("trailing test module", "fn a() {}\nfn b() {}\n#[cfg(test)]\nmod tests {\n // lots\n}\n", 2),
        ("no tests", "fn a() {}\n", 1),
        ("empty", "", 0),
        ("test-only helper", "fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn b() {}\n", 2),
        (
            "production code after a test module",
            "fn a() {}\n#[cfg(test)]\nmod a_tests {\n    #[test]\n    fn t() {\n    }\n}\n\nfn b() {}\n",
            3,
        ),
        (
            "a `}` in column 0 inside a test string",
            "#[cfg(test)]\nmod tests {\n    const FIX: &str = \"\n}\n\";\n    fn t() {}\n}\nfn b() {}\n",
            1,
        ),
        (
            "`#[cfg(test)]` inside a string is not an attribute",
            "const A: &str = \"\n#[cfg(test)]\nmod x {\";\nfn b() {}\n",
            4,
        ),
        ("doc comments go with their item", "/// tests\n#[cfg(test)]\nmod t {}\nfn b() {}\n", 1),
    ];
    for (why, source, expected) in cases {
        assert_eq!(production_lines(source), *expected, "{why}");
    }
}
