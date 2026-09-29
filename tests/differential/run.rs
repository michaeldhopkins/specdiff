//! Running a framework's own listing command.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Why a reference could not be produced here. A skip is printed; with
/// `SPECDIFF_DIFFERENTIAL_REQUIRE_ALL=1` (CI) it fails instead.
#[derive(Debug)]
pub struct Skip(pub String);

/// Run `cmd`, returning stdout. A program that is not installed is a skip; one that runs and
/// fails is a broken reference, and panics with its output.
pub fn output(cmd: &mut Command) -> Result<String, Skip> {
    let shown = format!("{cmd:?}");
    let out = match cmd.output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Skip(format!("{} is not installed", cmd.get_program().to_string_lossy())));
        }
        Err(e) => panic!("{shown}: {e}"),
    };
    assert!(
        out.status.success(),
        "{shown} failed ({}):\n{}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}


/// A scratch directory for a reference's build output, so a listing never writes into the
/// project it lists.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("differential").join(name);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    dir
}
