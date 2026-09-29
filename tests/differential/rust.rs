//! Rust: every test harness `cargo test` builds, asked for its tests with `--list`.
//!
//! Reference: `cargo test --no-run --all-features --message-format=json` names each test
//! executable (lib, bins, integration tests; the targets `cargo test` runs), then
//! `<exe> --list --format terse`. Validated: `--list` must include every `--ignored` test.
//! Doctests are deliberately out: rustdoc runs them, specdiff does not outline them, and no
//! executable here lists them.
//!
//! Attribution: a test path `a::b::tests::x` is walked from its target's root file through
//! `a.rs`/`a/mod.rs` and so on while such a file exists; the remaining segments are groups
//! inside that file. A file compiled into two targets (lib and bin) is one file, so the union
//! is taken. rstest's generated `name::case_N[_desc]` tests collapse to `name` with a count.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::{TestId, normalize};
use crate::outline::framework;
use crate::run::{Skip, output, scratch};

struct Harness {
    root_file: PathBuf,
    executable: PathBuf,
    package_dir: PathBuf,
}

fn harnesses(project: &Path, name: &str) -> Result<Vec<Harness>, Skip> {
    let json = output(
        Command::new("cargo")
            .args(["test", "--no-run", "--locked", "--all-features", "--message-format=json", "--manifest-path"])
            .arg(project.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", scratch(&format!("rust-{name}"))),
    )?;
    let mut found = Vec::new();
    for line in json.lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let is_test_harness = msg["reason"] == "compiler-artifact" && msg["profile"]["test"] == true;
        let (Some(exe), Some(src), Some(manifest)) =
            (msg["executable"].as_str(), msg["target"]["src_path"].as_str(), msg["manifest_path"].as_str())
        else {
            continue;
        };
        if is_test_harness {
            found.push(Harness {
                root_file: PathBuf::from(src),
                executable: PathBuf::from(exe),
                package_dir: Path::new(manifest).parent().map(Path::to_path_buf).unwrap_or_default(),
            });
        }
    }
    assert!(!found.is_empty(), "cargo built no test harness for {}", project.display());
    Ok(found)
}

fn listed(harness: &Harness, ignored_only: bool) -> Result<Vec<String>, Skip> {
    let mut cmd = Command::new(&harness.executable);
    cmd.args(["--list", "--format", "terse"]).current_dir(&harness.package_dir);
    if ignored_only {
        cmd.arg("--ignored");
    }
    Ok(output(&mut cmd)?.lines().filter_map(|l| l.strip_suffix(": test")).map(str::to_string).collect())
}

/// The file a module path lands in, and the segments left over inside it.
fn attribute<'a>(root_file: &Path, segments: &'a [&'a str]) -> (PathBuf, &'a [&'a str]) {
    let mut file = root_file.to_path_buf();
    let mut dir = root_file.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut used = 0;
    for seg in &segments[..segments.len().saturating_sub(1)] {
        let flat = dir.join(format!("{seg}.rs"));
        let nested = dir.join(seg).join("mod.rs");
        if flat.is_file() {
            file = flat;
            dir = dir.join(seg);
        } else if nested.is_file() {
            file = nested;
            dir = dir.join(seg);
        } else {
            break;
        }
        used += 1;
    }
    (file, &segments[used..])
}

fn rstest_case(segment: &str) -> bool {
    segment
        .strip_prefix("case_")
        .map(|rest| rest.split('_').next().unwrap_or_default())
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub fn reference(project: &Path, name: &str) -> Result<Vec<TestId>, Skip> {
    let norm = framework("rust_builtin").normalization.as_ref();
    let mut raw: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    for harness in harnesses(project, name)? {
        let all = listed(&harness, false)?;
        let ignored = listed(&harness, true)?;
        for test in &ignored {
            assert!(all.contains(test), "{test} is --ignored but missing from --list: the listing is incomplete");
        }
        for test in all {
            let segments: Vec<&str> = test.split("::").collect();
            let (file, rest) = attribute(&harness.root_file, &segments);
            let rel = file.strip_prefix(project).unwrap_or(&file).to_string_lossy().replace('\\', "/");
            raw.insert((rel, rest.iter().map(|s| (*s).to_string()).collect()));
        }
    }
    let mut cases: BTreeMap<(String, Vec<String>), usize> = BTreeMap::new();
    let mut ids = Vec::new();
    for (file, path) in raw {
        match path.split_last() {
            Some((last, parent)) if rstest_case(last) && !parent.is_empty() => {
                *cases.entry((file, parent.to_vec())).or_default() += 1;
            }
            _ => ids.push((file, path, None)),
        }
    }
    ids.extend(cases.into_iter().map(|((file, path), n)| (file, path, Some(n))));
    Ok(ids
        .into_iter()
        .map(|(file, path, cases)| TestId { file, path: path.iter().map(|s| normalize(s, norm)).collect(), cases })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rstest_cases_are_recognised_with_and_without_a_description() {
        assert!(rstest_case("case_1"));
        assert!(rstest_case("case_12_empty_input"));
        assert!(!rstest_case("case_"));
        assert!(!rstest_case("case_x"));
        assert!(!rstest_case("cases_1"));
    }
}
