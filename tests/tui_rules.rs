//! The "Terminal UIs" rule, checked. `tests/tui.toml` names the test covering everything the
//! TUI does that must be tested; this fails when a named test does not exist, a test spawns
//! the binary without `env!("CARGO_BIN_EXE_…")` and `env_clear()`, a test's PATH does not come
//! from its stub directory, `src/` runs a program by absolute path or one the manifest does not
//! list (the harness stubs exactly that list), or a test sleeps outside a deadline-bounded poll.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::visit::Visit;

fn rs_files(dir: &Path, skip: Option<&Path>, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if skip.is_some_and(|s| path.starts_with(s)) {
            continue;
        }
        if path.is_dir() {
            rs_files(&path, skip, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

fn parse(path: &Path) -> syn::File {
    let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    syn::parse_file(&source).unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()))
}

fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("test")
            || (a.path().is_ident("cfg")
                && a.parse_args::<syn::Meta>().is_ok_and(|m| m.path().is_ident("test")))
    })
}

fn last_segment(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

fn path_string(expr: &syn::Expr) -> String {
    match expr {
        syn::Expr::Path(p) => p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        _ => String::new(),
    }
}

fn str_lit(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
        _ => None,
    }
}

fn tokens(node: &impl ToTokens) -> String {
    node.to_token_stream().to_string().replace(' ', "")
}

#[derive(Default)]
struct TestFns(BTreeSet<String>);

impl<'a> Visit<'a> for TestFns {
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if f.attrs.iter().any(|a| a.path().is_ident("test")) {
            self.0.insert(f.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, f);
    }
}

struct Programs<'m> {
    file: String,
    allowed: &'m [String],
    out: Vec<String>,
}

impl<'a> Visit<'a> for Programs<'_> {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test_only(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !is_test_only(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
        let callee = path_string(&call.func);
        let spawns = callee.ends_with("Command::new") || callee.ends_with("CommandBuilder::new");
        let implied = match last_segment(&call.func) {
            Some(name) if name.starts_with("run_jj") => Some("jj".to_string()),
            Some(name) if name.starts_with("run_git") => Some("git".to_string()),
            _ => None,
        };
        let program = if spawns {
            match call.args.first().and_then(str_lit) {
                Some(p) => Some(p),
                None => {
                    self.out.push(format!("{}: runs a program chosen at run time: {}", self.file, tokens(call)));
                    None
                }
            }
        } else {
            implied
        };
        if let Some(program) = program {
            if program.starts_with('/') {
                self.out.push(format!("{}: runs {program} by absolute path, past every stub", self.file));
            } else if !self.allowed.contains(&program) {
                self.out.push(format!("{}: runs {program}, which tests/tui.toml `programs` does not list", self.file));
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

struct Harness {
    file: String,
    deadline_loops: Vec<bool>,
    out: Vec<String>,
}

impl Harness {
    fn check_fn(&mut self, name: &syn::Ident, body: &syn::Block) {
        let text = tokens(body);
        if text.contains("env!(\"CARGO_BIN_EXE_") && !text.contains(".env_clear()") {
            self.out.push(format!("{}: {name} spawns the binary without env_clear()", self.file));
        }
    }

    fn enter_loop(&mut self, body: &syn::Block) {
        self.deadline_loops.push(tokens(body).contains(".elapsed()"));
    }
}

impl<'a> Visit<'a> for Harness {
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        self.check_fn(&f.sig.ident, &f.block);
        syn::visit::visit_item_fn(self, f);
    }

    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        self.check_fn(&f.sig.ident, &f.block);
        syn::visit::visit_impl_item_fn(self, f);
    }

    fn visit_expr_loop(&mut self, l: &'a syn::ExprLoop) {
        self.enter_loop(&l.body);
        syn::visit::visit_expr_loop(self, l);
        self.deadline_loops.pop();
    }

    fn visit_expr_while(&mut self, l: &'a syn::ExprWhile) {
        self.enter_loop(&l.body);
        syn::visit::visit_expr_while(self, l);
        self.deadline_loops.pop();
    }

    fn visit_expr_for_loop(&mut self, l: &'a syn::ExprForLoop) {
        self.enter_loop(&l.body);
        syn::visit::visit_expr_for_loop(self, l);
        self.deadline_loops.pop();
    }

    fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
        match last_segment(&call.func).as_deref() {
            Some("sleep") if !self.deadline_loops.iter().any(|d| *d) => {
                self.out.push(format!("{}: sleeps outside a deadline-bounded poll: {}", self.file, tokens(call)));
            }
            Some("cargo_bin") => {
                self.out.push(format!("{}: spawns through cargo_bin, not env!(\"CARGO_BIN_EXE_…\")", self.file));
            }
            _ => {}
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'a syn::ExprMethodCall) {
        if call.method == "cargo_bin" {
            self.out.push(format!("{}: spawns through cargo_bin, not env!(\"CARGO_BIN_EXE_…\")", self.file));
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn path_violations(file: &str, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let on_a_pty = text.contains("portable_pty") && text.contains("env!(\"CARGO_BIN_EXE_");
    let sets_stub_path = ["\"PATH\",", "\"PATH\".into(),", "\"PATH\".to_string(),", "\"PATH\".to_owned(),"]
        .iter()
        .any(|key| text.contains(key));
    if on_a_pty && !sets_stub_path {
        out.push(format!("{file}: runs the binary on a pty without setting PATH to the stub directory"));
    }
    if text.contains("var(\"PATH\")") || text.contains("var_os(\"PATH\")") {
        out.push(format!("{file}: reads the inherited PATH"));
    }
    for key in ["\"PATH\",", "\"PATH\".into(),", "\"PATH\".to_string(),", "\"PATH\".to_owned(),"] {
        for (at, _) in text.match_indices(key) {
            let value = expression_at(&text[at + key.len()..]);
            if !value.contains("stubs.path()") {
                out.push(format!("{file}: sets PATH to {value}, not the stub directory's stubs.path()"));
            }
        }
    }
    out
}

fn expression_at(text: &str) -> &str {
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' | ',' | ';' if depth == 0 => return &text[..i],
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    text
}

fn listed_tests(manifest: &toml::Value) -> Vec<String> {
    let mut tests = Vec::new();
    for table in ["pty", "key", "view", "launches"] {
        for entry in manifest.get(table).and_then(toml::Value::as_array).into_iter().flatten() {
            if let Some(test) = entry.get("test").and_then(toml::Value::as_str) {
                tests.push(test.to_string());
            }
        }
    }
    tests
}

fn violations(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let manifest_path = root.join("tests/tui.toml");
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(&manifest_path).expect("tests/tui.toml"))
        .expect("tests/tui.toml parses");
    let programs: Vec<String> = manifest["programs"]
        .as_array()
        .expect("programs")
        .iter()
        .filter_map(|p| p.as_str().map(String::from))
        .collect();

    for listed in listed_tests(&manifest) {
        let Some((file, name)) = listed.split_once("::") else {
            out.push(format!("tests/tui.toml: {listed} is not <file>::<test>"));
            continue;
        };
        let path = root.join(file);
        if !path.exists() {
            out.push(format!("tests/tui.toml: {listed}: no file {file}"));
            continue;
        }
        let mut fns = TestFns::default();
        fns.visit_file(&parse(&path));
        if !fns.0.contains(name) {
            out.push(format!("tests/tui.toml: {listed}: no #[test] fn {name} in {file}"));
        }
    }

    let mut sources = Vec::new();
    rs_files(&root.join("src"), None, &mut sources);
    for path in sources {
        let mut visitor = Programs { file: rel(root, &path), allowed: &programs, out: Vec::new() };
        visitor.visit_file(&parse(&path));
        out.extend(visitor.out);
    }

    let mut tests = Vec::new();
    rs_files(&root.join("tests"), Some(&root.join("tests/fixtures")), &mut tests);
    for path in tests {
        let mut visitor = Harness { file: rel(root, &path), deadline_loops: Vec::new(), out: Vec::new() };
        let file = parse(&path);
        visitor.visit_file(&file);
        out.extend(visitor.out);
        out.extend(path_violations(&rel(root, &path), &tokens(&file)));
    }

    out.sort();
    out
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

#[test]
fn specdiff_meets_the_terminal_ui_rule() {
    let found = violations(Path::new(env!("CARGO_MANIFEST_DIR")));
    assert!(found.is_empty(), "Terminal UI rule violations:\n{}", found.join("\n"));
}

#[test]
fn the_check_finds_every_kind_of_violation_in_its_fixture() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tui_rules/bad");
    let found = violations(&fixture);
    let expected = [
        "src/main.rs: runs /usr/bin/security by absolute path, past every stub",
        "src/main.rs: runs a program chosen at run time: std::process::Command::new(editor)",
        "src/main.rs: runs open, which tests/tui.toml `programs` does not list",
        "tests/pty_without_path.rs: runs the binary on a pty without setting PATH to the stub directory",
        "tests/tui.rs: quits spawns the binary without env_clear()",
        "tests/tui.rs: reads the inherited PATH",
        "tests/tui.rs: sets PATH to \"/usr/bin\".to_string(), not the stub directory's stubs.path()",
        "tests/tui.rs: sets PATH to std::env::var(\"PATH\").unwrap(), not the stub directory's stubs.path()",
        "tests/tui.rs: sleeps outside a deadline-bounded poll: std::thread::sleep(Duration::from_millis(50))",
        "tests/tui.rs: spawns through cargo_bin, not env!(\"CARGO_BIN_EXE_…\")",
        "tests/tui.toml: tests/missing.rs::gone: no file tests/missing.rs",
        "tests/tui.toml: tests/tui.rs::resize_repaints: no #[test] fn resize_repaints in tests/tui.rs",
    ];
    assert_eq!(found, expected.map(String::from).to_vec());
}

#[test]
fn a_clean_fixture_has_no_violations() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tui_rules/good");
    assert_eq!(violations(&fixture), Vec::<String>::new());
}
