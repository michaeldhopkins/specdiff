//! Go: `go test -json -list '.*' ./...`, with `go list` mapping each package to its
//! directory.
//!
//! `-list` names only top-level functions (tests, benchmarks, fuzz targets and examples with
//! output), per package, and never subtests: `t.Run` names exist only at run time. So Go
//! compares at the depth the reference can see: the package directory and the top-level
//! function, with specdiff's side projected to its root nodes and no case counts. Validated:
//! every package `go list` reports must answer the listing. Build tags are not passed, so the
//! reference is the default build.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::model::{TestId, normalize};
use crate::outline::framework;
use crate::run::{Skip, output};

pub fn reference(project: &Path, _name: &str) -> Result<Vec<TestId>, Skip> {
    let packages = output(Command::new("go").args(["list", "-f", "{{.ImportPath}} {{.Dir}}", "./..."]).current_dir(project))?;
    let dirs: BTreeMap<String, String> = packages
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(import, dir)| {
            let rel = Path::new(dir).strip_prefix(project).unwrap_or(Path::new(dir));
            (import.to_string(), rel.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    let json = output(Command::new("go").args(["test", "-json", "-list", ".*", "./..."]).current_dir(project))?;
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
