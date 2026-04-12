mod render;

use crate::cli::Cli;
use crate::diff::types::FileDiff;
use crate::diff::{self, filter_file_diffs};
use crate::parse;
use crate::vcs;
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

enum AppEvent {
    Key(KeyEvent),
    FileChanged,
    Tick,
}

#[allow(clippy::struct_excessive_bools)]
struct AppState {
    file_diffs: Vec<FileDiff>,
    scroll: usize,
    changed_only: bool,
    filter: Option<String>,
    quit: bool,
    needs_redraw: bool,
}

pub fn run_watch(cli: &Cli) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine working directory")?;
    let vcs = crate::detect_vcs(&cwd)?;

    let base_rev = cli.base.clone().unwrap_or_else(|| "main".to_string());
    let head_rev = cli.head.clone().unwrap_or_else(|| vcs.default_head_rev().to_string());

    let merge_base = vcs.merge_base(&base_rev, &head_rev)
        .unwrap_or_else(|_| base_rev.clone());

    let file_diffs = compute_diffs(vcs.as_ref(), &merge_base, &head_rev, cli)?;

    let mut state = AppState {
        file_diffs,
        scroll: 0,
        changed_only: cli.changed_only,
        filter: cli.filter.clone(),
        quit: false,
        needs_redraw: true,
    };

    let (tx, rx) = mpsc::channel();

    let fs_tx = tx.clone();
    let watch_path = cwd.clone();
    let _debouncer = setup_watcher(watch_path, fs_tx)?;

    let tick_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            if tick_tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    let mut terminal = ratatui::init();
    let result = run_event_loop(&mut terminal, &mut state, &rx, vcs.as_ref(), &merge_base, &head_rev, cli);
    ratatui::restore();

    result
}

fn setup_watcher(path: PathBuf, tx: mpsc::Sender<AppEvent>) -> Result<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        move |events: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = events {
                let has_relevant = events.iter().any(|e| {
                    e.kind == DebouncedEventKind::Any
                        && e.path.extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| {
                                matches!(ext, "rb" | "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "exs")
                            })
                });
                if has_relevant {
                    let _ = tx.send(AppEvent::FileChanged);
                }
            }
        },
    )?;

    debouncer.watcher().watch(&path, notify::RecursiveMode::Recursive)?;

    Ok(debouncer)
}

fn run_event_loop(
    terminal: &mut DefaultTerminal,
    state: &mut AppState,
    rx: &mpsc::Receiver<AppEvent>,
    vcs: &dyn vcs::Vcs,
    merge_base: &str,
    head_rev: &str,
    cli: &Cli,
) -> Result<()> {
    loop {
        if state.quit {
            break;
        }

        if state.needs_redraw {
            let diffs = if let Some(pattern) = &state.filter {
                filter_file_diffs(state.file_diffs.clone(), pattern)
            } else {
                state.file_diffs.clone()
            };
            terminal.draw(|frame| {
                render::render(frame, &diffs, state.scroll, state.changed_only);
            })?;
            state.needs_redraw = false;
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                handle_key(state, key);
            }
        }

        match rx.try_recv() {
            Ok(AppEvent::FileChanged) | Ok(AppEvent::Tick) => {
                state.file_diffs = compute_diffs(vcs, merge_base, head_rev, cli)?;
                state.needs_redraw = true;
            }
            Ok(AppEvent::Key(key)) => {
                handle_key(state, key);
            }
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
        KeyCode::Down | KeyCode::Char('j') => {
            state.scroll = state.scroll.saturating_add(1);
            state.needs_redraw = true;
        }
        KeyCode::Up | KeyCode::Char('k') => {
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
}

fn compute_diffs(vcs: &dyn vcs::Vcs, merge_base: &str, head_rev: &str, cli: &Cli) -> Result<Vec<FileDiff>> {
    let changed = vcs.changed_files(merge_base, head_rev)?;

    let test_files: Vec<PathBuf> = changed
        .into_iter()
        .filter(|f| !parse::registry::frameworks_for_file(f).is_empty())
        .collect();

    let mut file_diffs = Vec::new();

    for path in &test_files {
        let frameworks = parse::registry::frameworks_for_file(path);
        let framework = if let Some(name) = &cli.framework {
            frameworks.iter().find(|f| f.name == *name).copied()
        } else {
            frameworks.first().copied()
        };

        let Some(framework) = framework else { continue };

        let base_source = vcs.file_at_revision(path, merge_base).ok();
        let head_source = vcs.file_at_revision(path, head_rev).ok();

        let rel_path = path.to_string_lossy();
        let base_tree = base_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, &rel_path, framework));
        let head_tree = head_source
            .as_deref()
            .and_then(|s| parse::engine::parse_file(s, &rel_path, framework));

        let base_nodes = base_tree.map(|t| t.root).unwrap_or_default();
        let head_nodes = head_tree.map(|t| t.root).unwrap_or_default();

        let nodes = diff::diff_spec_nodes(&base_nodes, &head_nodes);
        if !nodes.is_empty() {
            let display_path = parse::registry::normalize_file_path(&rel_path, framework);
            file_diffs.push(FileDiff { path: display_path, nodes });
        }
    }

    Ok(file_diffs)
}
