//! Source builds embed libgit2 instead of linking whatever Homebrew or the distribution has.
//!
//! A dynamic link to a Homebrew keg breaks when `brew cleanup` removes the old version: cargo's
//! cached build keeps the dead Cellar path and every binary fails to link.

use std::process::Command;

const FORBIDDEN: &[&str] = &[
    "libgit2",
    "/opt/homebrew/",
    "/usr/local/Cellar/",
    "/usr/local/opt/",
];

fn offending_lines(listing: &str) -> Vec<&str> {
    listing
        .lines()
        .filter(|l| FORBIDDEN.iter().any(|f| l.contains(f)))
        .collect()
}

#[test]
fn binary_does_not_link_a_system_libgit2() {
    let (tool, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("/usr/bin/otool", &["-L"])
    } else {
        ("/usr/bin/ldd", &[])
    };
    let out = Command::new(tool)
        .env_clear()
        .args(args)
        .arg(env!("CARGO_BIN_EXE_specdiff"))
        .output()
        .unwrap_or_else(|e| {
            panic!("{tool} is not available ({e}), so linked libraries cannot be listed")
        });
    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(!listing.is_empty(), "the linker listing was empty");
    let offending = offending_lines(&listing);
    assert!(
        offending.is_empty(),
        "specdiff links a system libgit2 or a package-manager path; enable git2's vendored-libgit2 feature:\n{}",
        offending.join("\n")
    );
}

#[test]
fn flags_homebrew_and_libgit2_paths() {
    let listing = "\t/opt/homebrew/opt/foo/lib/libfoo.dylib\n\t/usr/local/Cellar/x/1/lib/y.dylib\n\t/usr/local/opt/z/lib/z.dylib\n\t/lib/libgit2.so.1.9\n\t/usr/lib/libSystem.B.dylib\n";
    assert_eq!(offending_lines(listing).len(), 4);
    assert!(offending_lines("\t/usr/lib/libSystem.B.dylib\n").is_empty());
}
