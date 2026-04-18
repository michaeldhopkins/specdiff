mod render;

use crate::cli::Cli;
use crate::diff::filter_file_diffs;
use crate::diff::types::FileDiff;
use crate::parse;
use crate::pipeline::{self, DirectorySource, VcsSource};
use crate::vcs;
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

enum AppEvent {
    FileChanged,
    RegistryReady(crate::parse::shared::SharedExampleRegistry),
    Tick,
}

#[allow(clippy::struct_excessive_bools)]
struct AppState {
    file_diffs: Vec<FileDiff>,
    scroll: usize,
    section_offsets: Vec<usize>,
    changed_only: bool,
    filter: Option<String>,
    quit: bool,
    needs_redraw: bool,
    max_scroll: usize,
    cached_merge_base: Option<String>,
    merge_base_time: std::time::Instant,
    shared_registry: Option<crate::parse::shared::SharedExampleRegistry>,
}

enum WatchMode<'a> {
    Vcs {
        vcs: &'a dyn vcs::Vcs,
        base_rev: String,
        head_rev: String,
    },
    Directory {
        base: PathBuf,
        head: PathBuf,
    },
}

impl WatchMode<'_> {
    fn compute_diffs_fast(&self, state: &mut AppState, cli: &Cli) -> Result<Vec<FileDiff>> {
        let mb = self.resolve_merge_base(state);
        self.with_source(mb.as_deref(), |source| pipeline::diff_files_fast(source, cli))
    }

    fn compute_diffs_with_registry(
        &self,
        state: &mut AppState,
        cli: &Cli,
        registry: &crate::parse::shared::SharedExampleRegistry,
    ) -> Result<Vec<FileDiff>> {
        let mb = self.resolve_merge_base(state);
        self.with_source(mb.as_deref(), |source| pipeline::diff_files_with_registry(source, cli, registry))
    }

    fn needs_shared_scan(&self, state: &mut AppState, cli: &Cli) -> bool {
        let mb = self.resolve_merge_base(state);
        self.with_source(mb.as_deref(), |source| {
            let paths = source.list_files().unwrap_or_default();
            Ok(pipeline::changed_files_need_shared_scan(&paths, source, cli))
        })
        .unwrap_or(false)
    }

    fn build_registry(&self, state: &mut AppState, cli: &Cli) -> crate::parse::shared::SharedExampleRegistry {
        let mb = self.resolve_merge_base(state);
        self.with_source(mb.as_deref(), |source| {
            let all_paths = source.list_files().unwrap_or_default();
            let shared_paths = source.list_shared_files_all();
            let all_scannable: Vec<String> = {
                let mut set: std::collections::BTreeSet<String> = all_paths.into_iter().collect();
                set.extend(shared_paths);
                set.into_iter().collect()
            };
            Ok(pipeline::build_shared_registry(source, &all_scannable, cli, |s, path| s.read_head(path)))
        })
        .unwrap_or_default()
    }

    fn resolve_merge_base(&self, state: &mut AppState) -> Option<String> {
        match self {
            WatchMode::Vcs { vcs, base_rev, head_rev } => {
                let stale = state.merge_base_time.elapsed() > Duration::from_secs(2);
                if !stale
                    && let Some(cached) = &state.cached_merge_base
                {
                    return Some(cached.clone());
                }
                let mb = vcs.merge_base(base_rev, head_rev)
                    .unwrap_or_else(|_| base_rev.clone());
                state.cached_merge_base = Some(mb.clone());
                state.merge_base_time = std::time::Instant::now();
                Some(mb)
            }
            WatchMode::Directory { .. } => None,
        }
    }

    fn with_source<T>(&self, merge_base: Option<&str>, f: impl FnOnce(&dyn pipeline::FileSource) -> Result<T>) -> Result<T> {
        match self {
            WatchMode::Vcs { vcs, base_rev, head_rev } => {
                let mb = merge_base.unwrap_or(base_rev);
                let changed = vcs.changed_files(mb, head_rev)?;
                let test_files: Vec<PathBuf> = changed
                    .into_iter()
                    .filter(|f| !parse::registry::frameworks_for_file(f).is_empty())
                    .collect();
                let source = VcsSource {
                    vcs: *vcs,
                    files: test_files,
                    merge_base: mb.to_string(),
                    head_rev: head_rev.clone(),
                };
                f(&source)
            }
            WatchMode::Directory { base, head } => {
                let source = DirectorySource {
                    base: base.clone(),
                    head: head.clone(),
                };
                f(&source)
            }
        }
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        match self {
            WatchMode::Vcs { .. } => {
                std::env::current_dir().map(|c| vec![c]).unwrap_or_default()
            }
            WatchMode::Directory { base, head } => {
                let mut paths = vec![head.clone()];
                if base != head {
                    paths.push(base.clone());
                }
                paths
            }
        }
    }
}

pub fn run_watch(cli: &Cli) -> Result<()> {
    match (&cli.base_dir, &cli.head_dir) {
        (Some(base), Some(head)) => run_watch_directory(base, head, cli),
        (Some(_), None) | (None, Some(_)) => {
            anyhow::bail!("--base-dir and --head-dir must be used together")
        }
        (None, None) => run_watch_vcs(cli),
    }
}

fn run_watch_vcs(cli: &Cli) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine working directory")?;
    let vcs = crate::vcs::detect(&cwd)?;

    let base_rev = cli.base.clone().unwrap_or_else(|| vcs.default_base_rev());
    let head_rev = cli.head.clone().unwrap_or_else(|| vcs.default_head_rev().to_string());

    let mode = WatchMode::Vcs {
        vcs: vcs.as_ref(),
        base_rev,
        head_rev,
    };

    run_watch_loop(&mode, cli)
}

fn run_watch_directory(base: &str, head: &str, cli: &Cli) -> Result<()> {
    let mode = WatchMode::Directory {
        base: PathBuf::from(base),
        head: PathBuf::from(head),
    };
    run_watch_loop(&mode, cli)
}

fn run_watch_loop(mode: &WatchMode<'_>, cli: &Cli) -> Result<()> {
    let mut state = AppState {
        file_diffs: vec![],
        scroll: 0,
        section_offsets: vec![],
        max_scroll: 0,
        changed_only: cli.changed_only,
        filter: cli.filter.clone(),
        quit: false,
        needs_redraw: true,
        cached_merge_base: None,
        merge_base_time: std::time::Instant::now(),
        shared_registry: None,
    };

    state.file_diffs = mode.compute_diffs_fast(&mut state, cli)?;

    let (tx, rx) = mpsc::channel();

    let fs_tx = tx.clone();
    let _debouncer = setup_watcher(mode.watch_paths(), fs_tx)?;

    let tick_tx = tx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        if tick_tx.send(AppEvent::Tick).is_err() {
            break;
        }
    });

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        prev_hook(info);
    }));

    let mut terminal = ratatui::init();

    if mode.needs_shared_scan(&mut state, cli) {
        let filtered;
        let diffs: &[FileDiff] = if let Some(pattern) = &state.filter {
            filtered = filter_file_diffs(state.file_diffs.clone(), pattern);
            &filtered
        } else {
            &state.file_diffs
        };
        terminal.draw(|frame| {
            render::render(frame, diffs, state.scroll, state.changed_only);
        })?;

        let registry = mode.build_registry(&mut state, cli);
        if let Ok(diffs) = mode.compute_diffs_with_registry(&mut state, cli, &registry) {
            state.file_diffs = diffs;
            state.shared_registry = Some(registry);
            state.needs_redraw = true;
        }
    }

    let result = run_event_loop(&mut terminal, &mut state, &rx, mode, cli);
    ratatui::restore();

    result
}

fn setup_watcher(
    paths: Vec<PathBuf>,
    tx: mpsc::Sender<AppEvent>,
) -> Result<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        move |events: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = events {
                let dominated = events.iter().any(|e| {
                    e.kind == DebouncedEventKind::Any && is_relevant_path(&e.path)
                });
                if dominated {
                    let _ = tx.send(AppEvent::FileChanged);
                }
            }
        },
    )?;

    for path in paths {
        if path.exists() {
            debouncer
                .watcher()
                .watch(&path, notify::RecursiveMode::Recursive)?;
        }
    }

    Ok(debouncer)
}

fn run_event_loop(
    terminal: &mut DefaultTerminal,
    state: &mut AppState,
    rx: &mpsc::Receiver<AppEvent>,
    mode: &WatchMode<'_>,
    cli: &Cli,
) -> Result<()> {
    loop {
        if state.quit {
            break;
        }

        if state.needs_redraw {
            let filtered;
            let diffs: &[FileDiff] = if let Some(pattern) = &state.filter {
                filtered = filter_file_diffs(state.file_diffs.clone(), pattern);
                &filtered
            } else {
                &state.file_diffs
            };
            let mut result = render::RenderResult { section_offsets: vec![], max_scroll: 0 };
            terminal.draw(|frame| {
                result = render::render(frame, diffs, state.scroll, state.changed_only);
            })?;
            state.section_offsets = result.section_offsets;
            state.max_scroll = result.max_scroll;
            state.scroll = state.scroll.min(state.max_scroll);
            state.needs_redraw = false;
        }

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            handle_key(state, key);
        }

        match rx.try_recv() {
            Ok(AppEvent::FileChanged) => {
                state.cached_merge_base = None;
                let reg = state.shared_registry.clone();
                let diffs = if let Some(reg) = &reg {
                    mode.compute_diffs_with_registry(state, cli, reg)?
                } else {
                    mode.compute_diffs_fast(state, cli)?
                };
                state.file_diffs = diffs;
                state.needs_redraw = true;
            }
            Ok(AppEvent::RegistryReady(registry)) => {
                if let Ok(diffs) = mode.compute_diffs_with_registry(state, cli, &registry) {
                    state.file_diffs = diffs;
                    state.shared_registry = Some(registry);
                    state.needs_redraw = true;
                }
            }
            Ok(AppEvent::Tick) => {}
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => break,
        }
    }

    Ok(())
}

fn handle_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => state.quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => state.quit = true,
        KeyCode::Char('j') => {
            state.scroll = next_section(state.scroll, &state.section_offsets);
            state.needs_redraw = true;
        }
        KeyCode::Char('k') => {
            state.scroll = prev_section(state.scroll, &state.section_offsets);
            state.needs_redraw = true;
        }
        KeyCode::Down => {
            state.scroll = state.scroll.saturating_add(1);
            state.needs_redraw = true;
        }
        KeyCode::Up => {
            state.scroll = state.scroll.saturating_sub(1);
            state.needs_redraw = true;
        }
        KeyCode::PageDown => {
            state.scroll = state.scroll.saturating_add(20);
            state.needs_redraw = true;
        }
        KeyCode::PageUp => {
            state.scroll = state.scroll.saturating_sub(20);
            state.needs_redraw = true;
        }
        KeyCode::Home | KeyCode::Char('g') => {
            state.scroll = 0;
            state.needs_redraw = true;
        }
        KeyCode::End | KeyCode::Char('G') => {
            state.scroll = usize::MAX;
            state.needs_redraw = true;
        }
        KeyCode::Char('c') => {
            state.changed_only = !state.changed_only;
            state.needs_redraw = true;
        }
        _ => {}
    }
    state.scroll = state.scroll.min(state.max_scroll);
}

fn next_section(current: usize, offsets: &[usize]) -> usize {
    offsets
        .iter()
        .find(|&&o| o > current)
        .copied()
        .unwrap_or(current)
}

fn prev_section(current: usize, offsets: &[usize]) -> usize {
    offsets
        .iter()
        .rev()
        .find(|&&o| o < current)
        .copied()
        .unwrap_or(0)
}

fn is_relevant_path(path: &std::path::Path) -> bool {
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
    use std::path::Path;

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
    fn next_section_jumps_forward() {
        let offsets = vec![0, 10, 20];
        assert_eq!(next_section(0, &offsets), 10);
        assert_eq!(next_section(10, &offsets), 20);
        assert_eq!(next_section(20, &offsets), 20);
    }

    #[test]
    fn prev_section_jumps_backward() {
        let offsets = vec![0, 10, 20];
        assert_eq!(prev_section(20, &offsets), 10);
        assert_eq!(prev_section(10, &offsets), 0);
        assert_eq!(prev_section(0, &offsets), 0);
    }
}
