use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn fixtures_dir() -> Option<&'static Path> {
    let dir = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../specdiff-tests/fixtures"
    ));
    if dir.exists() { Some(dir) } else { None }
}

#[test]
fn cli_version() {
    Command::cargo_bin("spec-diff")
        .expect("binary")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("spec-diff"));
}

#[test]
fn cli_help() {
    Command::cargo_bin("spec-diff")
        .expect("binary")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("test outline"));
}

#[test]
fn cli_no_args_in_non_repo_errors() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    Command::cargo_bin("spec-diff")
        .expect("binary")
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no git repository"));
}

#[test]
fn cli_tree_format_rspec_fixtures() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("skipping: specdiff-tests not found");
        return;
    };
    let base = fixtures.join("rspec/base");
    let head = fixtures.join("rspec/head");

    Command::cargo_bin("spec-diff")
        .expect("binary")
        .args(["--base-dir", base.to_str().expect("utf8"), "--head-dir", head.to_str().expect("utf8")])
        .assert()
        .success()
        .stdout(predicate::str::contains("validates email format"))
        .stdout(predicate::str::contains("+"));
}

#[test]
fn cli_json_format_rspec_fixtures() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("skipping: specdiff-tests not found");
        return;
    };
    let base = fixtures.join("rspec/base");
    let head = fixtures.join("rspec/head");

    Command::cargo_bin("spec-diff")
        .expect("binary")
        .args([
            "--base-dir", base.to_str().expect("utf8"),
            "--head-dir", head.to_str().expect("utf8"),
            "--format", "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"Modified\""))
        .stdout(predicate::str::contains("validates email format"));
}

#[test]
fn cli_compact_format_rust_fixtures() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("skipping: specdiff-tests not found");
        return;
    };
    let base = fixtures.join("rust_builtin/base");
    let head = fixtures.join("rust_builtin/head");

    Command::cargo_bin("spec-diff")
        .expect("binary")
        .args([
            "--base-dir", base.to_str().expect("utf8"),
            "--head-dir", head.to_str().expect("utf8"),
            "--format", "compact",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("- lib > tests > subtraction"))
        .stdout(predicate::str::contains("+ lib > tests > multiplication"));
}

#[test]
fn cli_changed_only_flag() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("skipping: specdiff-tests not found");
        return;
    };
    let base = fixtures.join("rspec/base");
    let head = fixtures.join("rspec/head");

    let output = Command::cargo_bin("spec-diff")
        .expect("binary")
        .args([
            "--base-dir", base.to_str().expect("utf8"),
            "--head-dir", head.to_str().expect("utf8"),
            "--changed-only",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(!stdout.contains("   associations\n       has many posts\n"), "unchanged associations group should be filtered");
}

#[test]
fn cli_base_dir_without_head_dir_errors() {
    Command::cargo_bin("spec-diff")
        .expect("binary")
        .args(["--base-dir", "/tmp/nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be used together"));
}

#[test]
fn cli_filter_flag() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("skipping: specdiff-tests not found");
        return;
    };
    let base = fixtures.join("rspec/base");
    let head = fixtures.join("rspec/head");

    let output = Command::cargo_bin("spec-diff")
        .expect("binary")
        .args([
            "--base-dir", base.to_str().expect("utf8"),
            "--head-dir", head.to_str().expect("utf8"),
            "--filter", "admin",
            "--no-color",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("admin"), "should show admin specs");
    assert!(!stdout.contains("validates email"), "should not show non-matching specs");
}
