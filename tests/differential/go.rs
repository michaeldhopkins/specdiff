//! Go: `go test -json -list '.*' ./...`, with `go list` mapping each package to its
//! directory.
//!
//! `-list` names only top-level functions (tests, benchmarks, fuzz targets and examples with
//! output), per package, and never subtests: `t.Run` names exist only at run time. So Go
//! compares at the depth the reference can see: the package directory and the top-level
//! function, with specdiff's side projected to its root nodes and no case counts. Validated:
//! every package `go list` reports must answer the listing. Build tags are not passed, so the
//! reference is the default build. Not quite non-executing: `-list` runs `TestMain`, so a
//! package whose `TestMain` fails (a smoke suite probing a server) makes the project a skip.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::model::{TestId, normalize};
use crate::outline::{framework, project_files};
use crate::run::{Skip, output, output_or_skip};

/// Every Go module in the project: `./...` stops at a nested `go.mod`, so each is listed on
/// its own, or a nested module's tests go missing from the reference.
pub fn reference(project: &Path, name: &str) -> Result<Vec<TestId>, Skip> {
    let mut ids = Vec::new();
    for module in project_files(project).iter().filter(|f| f.rsplit('/').next() == Some("go.mod")) {
        let dir = project.join(Path::new(module).parent().unwrap_or(Path::new("")));
        ids.extend(module_reference(project, &dir, name)?);
    }
    Ok(ids)
}

fn module_reference(project: &Path, module: &Path, _name: &str) -> Result<Vec<TestId>, Skip> {
    let packages = output(Command::new("go").args(["list", "-f", "{{.ImportPath}} {{.Dir}}", "./..."]).current_dir(module))?;
    let dirs: BTreeMap<String, String> = packages
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(import, dir)| {
            let rel = Path::new(dir).strip_prefix(project).unwrap_or(Path::new(dir));
            (import.to_string(), rel.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    let json = output_or_skip(
        Command::new("go").args(["test", "-json", "-list", ".*", "./..."]).current_dir(module),
        "go test -list runs each package's TestMain, which can fail without running a test",
    )?;
    let norm = framework("go_testing").normalization.as_ref();
    let mut answered = std::collections::BTreeSet::new();
    let mut ids = Vec::new();
    for line in json.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(package) = event["Package"].as_str() else { continue };
        answered.insert(package.to_string());
        if event["Action"] != "output" {
            continue;
        }
        let text = event["Output"].as_str().unwrap_or_default().trim();
        let is_name = !text.is_empty() && !text.contains(char::is_whitespace);
        if !is_name {
            continue;
        }
        let dir = dirs.get(package).unwrap_or_else(|| panic!("go list did not report {package}"));
        ids.push(TestId { file: dir.clone(), path: vec![normalize(text, norm)], cases: None });
    }
    for package in dirs.keys() {
        assert!(answered.contains(package), "go test -list said nothing for {package}: the listing is incomplete");
    }
    Ok(ids)
}

/// specdiff's side at the reference's depth: each root node under its file's directory.
pub fn project(outlined: &[(String, Vec<specdiff::parse::SpecNode>)]) -> Vec<TestId> {
    outlined
        .iter()
        .flat_map(|(file, roots)| {
            let dir = Path::new(file).parent().map(|d| d.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            roots.iter().map(move |n| TestId { file: dir.clone(), path: vec![n.name.clone()], cases: None })
        })
        .collect()
}
