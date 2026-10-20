use anyhow::Result;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, DebouncedEventKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn setup_watcher(
    paths: Vec<PathBuf>,
    on_change: impl Fn() + Send + 'static,
) -> Result<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        move |events: Result<Vec<DebouncedEvent>, notify::Error>| {
            if let Ok(events) = events
                && is_relevant_batch(&events)
            {
                on_change();
            }
        },
    )?;

    for path in paths {
        if path.exists() {
            debouncer.watcher().watch(&path, notify::RecursiveMode::Recursive)?;
        }
    }

    Ok(debouncer)
}

pub fn is_relevant_batch(events: &[DebouncedEvent]) -> bool {
    events.iter().any(|e| e.kind == DebouncedEventKind::Any && is_relevant_path(&e.path))
}

pub fn is_relevant_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.contains(".git/refs") || s.contains(".jj/repo") {
        return true;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(ext, "rb" | "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "exs" | "java" | "php")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::mpsc;

    fn event(path: &str, kind: DebouncedEventKind) -> DebouncedEvent {
        DebouncedEvent::new(PathBuf::from(path), kind)
    }

    fn arbitrary_event() -> impl Strategy<Value = (String, bool)> {
        ("[a-zA-Z0-9_./-]{0,48}", proptest::bool::ANY)
    }

    fn events(raw: &[(String, bool)]) -> Vec<DebouncedEvent> {
        raw.iter()
            .map(|(path, settled)| {
                event(
                    path,
                    if *settled {
                        DebouncedEventKind::Any
                    } else {
                        DebouncedEventKind::AnyContinuous
                    },
                )
            })
            .collect()
    }

    #[test]
    fn relevant_path_test_files() {
        assert!(is_relevant_path(Path::new("spec/models/user_spec.rb")));
        assert!(is_relevant_path(Path::new("src/lib.rs")));
        assert!(is_relevant_path(Path::new("tests/test_user.py")));
        assert!(is_relevant_path(Path::new("user.test.js")));
        assert!(is_relevant_path(Path::new("user.test.tsx")));
        assert!(is_relevant_path(Path::new("user_test.go")));
        assert!(is_relevant_path(Path::new("test/user_test.exs")));
        assert!(is_relevant_path(Path::new("tests/UserTest.java")));
        assert!(is_relevant_path(Path::new("tests/UserTest.php")));
    }

    #[test]
    fn relevant_path_vcs_refs() {
        assert!(is_relevant_path(Path::new("/repo/.git/refs/heads/main")));
        assert!(is_relevant_path(Path::new("/repo/.jj/repo/op_heads/abc")));
    }

    #[test]
    fn irrelevant_paths() {
        assert!(!is_relevant_path(Path::new("Cargo.toml")));
        assert!(!is_relevant_path(Path::new("README.md")));
        assert!(!is_relevant_path(Path::new("src/main.css")));
        assert!(!is_relevant_path(Path::new(".git/index")));
        assert!(!is_relevant_path(Path::new(".git/COMMIT_EDITMSG")));
    }

    #[test]
    fn a_batch_counts_only_settled_events_on_relevant_paths() {
        assert!(is_relevant_batch(&[event("spec/a_spec.rb", DebouncedEventKind::Any)]));
        assert!(is_relevant_batch(&[
            event("README.md", DebouncedEventKind::Any),
            event("spec/a_spec.rb", DebouncedEventKind::Any),
        ]));
        assert!(!is_relevant_batch(&[event("spec/a_spec.rb", DebouncedEventKind::AnyContinuous)]));
        assert!(!is_relevant_batch(&[event("README.md", DebouncedEventKind::Any)]));
        assert!(!is_relevant_batch(&[]));
    }

    #[test]
    fn saving_a_watched_test_file_signals_a_change() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical");
        let (tx, rx) = mpsc::channel();
        let _watcher = setup_watcher(vec![root.clone(), root.join("absent")], move || {
            let _ = tx.send(());
        })
        .expect("watcher");
        std::fs::write(root.join("user_spec.rb"), "RSpec.describe User do\nend\n").expect("write");
        assert!(rx.recv_timeout(Duration::from_secs(10)).is_ok(), "no change signalled");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn a_batch_is_relevant_iff_either_of_its_two_halves_is_relevant(
            first in proptest::collection::vec(arbitrary_event(), 0..5),
            second in proptest::collection::vec(arbitrary_event(), 0..5),
        ) {
            let (left, right) = (events(&first), events(&second));
            let mut combined = left.clone();
            combined.extend(right.iter().cloned());
            prop_assert_eq!(
                is_relevant_batch(&combined),
                is_relevant_batch(&left) || is_relevant_batch(&right)
            );
        }

        #[test]
        fn a_continuous_only_batch_is_never_relevant(
            paths in proptest::collection::vec("[a-zA-Z0-9_./-]{0,48}", 0..6),
        ) {
            let batch: Vec<DebouncedEvent> =
                paths.iter().map(|p| event(p, DebouncedEventKind::AnyContinuous)).collect();
            prop_assert!(!is_relevant_batch(&batch));
        }

        #[test]
        fn a_path_under_a_plain_directory_keeps_its_relevance(
            dir in "[a-z]{1,6}",
            name in "[a-zA-Z0-9_./-]{0,48}",
        ) {
            let nested = Path::new(&dir).join(&name);
            prop_assert_eq!(is_relevant_path(&nested), is_relevant_path(Path::new(&name)));
        }

        #[test]
        fn a_dotted_file_under_a_plain_directory_is_relevant_iff_its_extension_is_watched(
            dir in "[a-z]{1,6}",
            stem in "[a-z0-9_]{1,8}",
            (ext, watched) in proptest::sample::select(vec![
                ("rb", true), ("rs", true), ("py", true), ("js", true), ("jsx", true),
                ("ts", true), ("tsx", true), ("go", true), ("exs", true), ("java", true),
                ("php", true), ("md", false), ("txt", false), ("toml", false), ("css", false),
                ("lock", false), ("html", false), ("json", false), ("yaml", false), ("log", false),
            ]),
        ) {
            let path = Path::new(&dir).join(format!("{stem}.{ext}"));
            prop_assert_eq!(is_relevant_path(&path), watched);
        }
    }
}
