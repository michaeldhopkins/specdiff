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
    use std::sync::mpsc;

    fn event(path: &str, kind: DebouncedEventKind) -> DebouncedEvent {
        DebouncedEvent::new(PathBuf::from(path), kind)
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
}
