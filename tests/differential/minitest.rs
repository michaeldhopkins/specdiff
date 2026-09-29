//! Minitest: load every test file and ask each runnable class for its `runnable_methods`,
//! which is exactly what `Minitest.run` would run. `minitest_list.rb` does that without
//! running anything (it disables autorun). Minitest has no listing flag of its own.
//!
//! Test files are Rails' and `Rake::TestTask`'s conventions, `test/**/*_test.rb` and
//! `test/**/test_*.rb`, found without asking specdiff. `test/` goes on the load path, as
//! `rake test` does. A project that needs Rails booted to load its tests is out of reach
//! here; the script fails and the harness reports it.
//!
//! Identifier: the test file, the class name split on `::` (a spec's nested describes are
//! `Outer::inner`), then the method, each through minitest.toml's normalisation. A spec's
//! `test_0001_name` is minitest's encoding of `it "name"`, so the counter is stripped first.

use std::path::Path;
use std::process::Command;

use crate::model::{TestId, normalize};
use crate::outline::{framework, project_files};
use crate::run::{Skip, output};

fn spec_name(method: &str) -> Option<&str> {
    let rest = method.strip_prefix("test_")?;
    let (counter, name) = rest.split_once('_')?;
    (counter.len() == 4 && counter.bytes().all(|b| b.is_ascii_digit())).then_some(name)
}

pub fn reference(project: &Path, _name: &str) -> Result<Vec<TestId>, Skip> {
    let patterns = ["test/**/*_test.rb", "test/**/test_*.rb"].map(|p| glob::Pattern::new(p).expect("glob"));
    let files: Vec<String> =
        project_files(project).into_iter().filter(|f| patterns.iter().any(|p| p.matches(f))).collect();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/differential/minitest_list.rb");
    let listed = output(Command::new("ruby").arg(script).arg(project).args(&files).current_dir(project))?;
    let norm = framework("minitest").normalization.as_ref();
    let mut ids = Vec::new();
    for line in listed.lines() {
        let test: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        let field = |k: &str| test[k].as_str().unwrap_or_default().to_string();
        let file = field("file");
        let rel = Path::new(&file).strip_prefix(project).map_or(file.clone(), |p| p.to_string_lossy().replace('\\', "/"));
        let method = field("method");
        let mut path: Vec<String> = field("class").split("::").map(|s| normalize(s, norm)).collect();
        path.push(spec_name(&method).map_or_else(|| normalize(&method, norm), str::to_string));
        ids.push(TestId { file: rel, path, cases: None });
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_four_digit_counter_marks_a_spec() {
        assert_eq!(spec_name("test_0001_starts empty"), Some("starts empty"));
        assert_eq!(spec_name("test_valid_user"), None);
        assert_eq!(spec_name("test_12_x"), None);
    }
}
