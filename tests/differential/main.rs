//! Differential testing: does specdiff's outline agree with each framework's own listing of
//! the tests that exist? The framework is the reference; see AGENTS.md "Differential
//! testing". Every test here is `#[ignore]`d because each builds or boots a foreign
//! toolchain; run them with `cargo test --test differential -- --ignored --nocapture`.
//!
//! A framework whose toolchain is absent is skipped with a line on stderr, or fails when
//! `SPECDIFF_DIFFERENTIAL_REQUIRE_ALL=1` (CI sets it). Any difference that no entry in
//! `known.toml` explains fails, and so does a class whose entries explained nothing in a run over the
//! checked-in corpus (so the list cannot rot).

mod go;
mod minitest;
mod model;
mod outline;
mod run;
mod rust;

use std::path::{Path, PathBuf};

use model::{Report, TestId, load_known};
use run::Skip;

type Reference = fn(&Path, &str) -> Result<Vec<TestId>, Skip>;

fn reference_for(family: &str) -> Option<Reference> {
    match family {
        "rust" => Some(rust::reference),
        "go" => Some(go::reference),
        "minitest" => Some(minitest::reference),
        _ => None,
    }
}

/// specdiff's side, at the depth the family's reference can see.
fn outlined(family: &str, project: &Path) -> Vec<TestId> {
    match family {
        "go" => go::project(&outline::trees(project, family)),
        _ => outline::outline(project, family),
    }
}

fn corpus(family: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/differential").join(family)
}

fn known() -> Vec<model::Known> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/differential/known.toml");
    load_known(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

fn require_all() -> bool {
    std::env::var("SPECDIFF_DIFFERENTIAL_REQUIRE_ALL").is_ok_and(|v| v == "1")
}

/// Compare one project; `None` when its reference was skipped.
fn compare_project(report: &mut Report, family: &str, project: &Path, known: &[model::Known]) -> Option<()> {
    let project = project.canonicalize().unwrap_or_else(|e| panic!("{}: {e}", project.display()));
    let name = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let reference = reference_for(family).unwrap_or_else(|| panic!("no reference for {family}"));
    match reference(&project, &name) {
        Ok(listed) => {
            report.add(family, &listed, &outlined(family, &project), known);
            Some(())
        }
        Err(Skip(why)) => {
            assert!(!require_all(), "differential {family} cannot run: {why}");
            eprintln!("differential: SKIPPED {family} ({}): {why}", project.display());
            None
        }
    }
}

fn check_corpus(family: &str) {
    let known = known();
    let mut report = Report::default();
    let ran = compare_project(&mut report, family, &corpus(family), &known).is_some();
    eprintln!("{}", report.render());
    assert!(report.unexplained.is_empty(), "unexplained differences; see the report above");
    if ran {
        let unused: Vec<String> =
            report.unused(&[family], &known).iter().map(|k| format!("{} ({})", k.class, k.reason)).collect();
        assert!(unused.is_empty(), "known.toml classes that explained nothing in the corpus: {unused:?}");
    }
}

#[test]
#[ignore = "builds the corpus crate with cargo"]
fn rust_outline_agrees_with_cargo_test_list() {
    check_corpus("rust");
}

/// Real projects, outside the repository: `SPECDIFF_DIFFERENTIAL_PROJECTS` is a
/// comma-separated list of `family=path`. Only listing commands run in them, with build output
/// sent to a scratch directory. The report is the point; unexplained differences still fail.
#[test]
#[ignore = "reads projects named in SPECDIFF_DIFFERENTIAL_PROJECTS"]
fn real_projects_agree_with_their_frameworks() {
    let Ok(list) = std::env::var("SPECDIFF_DIFFERENTIAL_PROJECTS") else {
        eprintln!("differential: SPECDIFF_DIFFERENTIAL_PROJECTS is not set; nothing to compare");
        return;
    };
    let known = known();
    let mut report = Report::default();
    for entry in list.split(',').filter(|e| !e.trim().is_empty()) {
        let (family, path) = entry.split_once('=').unwrap_or_else(|| panic!("{entry} is not family=path"));
        compare_project(&mut report, family.trim(), Path::new(path.trim()), &known);
    }
    eprintln!("{}", report.render());
    assert!(report.unexplained.is_empty(), "unexplained differences; see the report above");
}

#[test]
#[ignore = "runs go test -list over the corpus"]
fn go_outline_agrees_with_go_test_list() {
    check_corpus("go");
}

#[test]
#[ignore = "loads the corpus with ruby and minitest"]
fn minitest_outline_agrees_with_runnable_methods() {
    check_corpus("minitest");
}
