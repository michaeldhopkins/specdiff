mod input;
mod render;
mod state;
mod watch;

use crate::cli::Cli;
use crate::diff::filter_file_diffs;
use crate::diff::types::FileDiff;
use crate::parse;
use crate::parse::shared::SharedExampleRegistry;
use crate::pipeline::{self, DirectorySource, VcsSource};
use crate::vcs;
use anyhow::{Context, Result};
use crossterm::event;
use ratatui::DefaultTerminal;
use state::{AppState, Effect};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

enum AppEvent {
    FileChanged,
    Tick,
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
        registry: &SharedExampleRegistry,
    ) -> Result<Vec<FileDiff>> {
        let mb = self.resolve_merge_base(state);
        self.with_source(mb.as_deref(), |source| pipeline::diff_files_with_registry(source, cli, registry))
    }

    fn recompute(&self, state: &mut AppState, cli: &Cli) -> Result<Vec<FileDiff>> {
        match state.shared_registry.clone() {
            Some(registry) => self.compute_diffs_with_registry(state, cli, &registry),
            None => self.compute_diffs_fast(state, cli),
        }
    }

    fn needs_shared_scan(&self, state: &mut AppState, cli: &Cli) -> bool {
        let mb = self.resolve_merge_base(state);
        self.with_source(mb.as_deref(), |source| {
            let paths = source.list_files().unwrap_or_default();
            Ok(pipeline::changed_files_need_shared_scan(&paths, source, cli))
        })
        .unwrap_or(false)
    }

    fn build_registry(&self, state: &mut AppState, cli: &Cli) -> SharedExampleRegistry {
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
                let now = Instant::now();
                if let Some(cached) = state.cached_merge_base(now) {
                    return Some(cached.clone());
                }
                let mb = vcs.merge_base(base_rev, head_rev).unwrap_or_else(|_| base_rev.clone());
                state.remember_merge_base(mb.clone(), now);
                Some(mb)
            }
            WatchMode::Directory { .. } => None,
        }
    }

    fn merge_base_moved(&self, state: &AppState) -> bool {
        match self {
            WatchMode::Vcs { vcs, base_rev, head_rev } => {
                let fresh = vcs.merge_base(base_rev, head_rev).unwrap_or_else(|_| base_rev.clone());
                state.merge_base_moved(&fresh)
            }
            WatchMode::Directory { .. } => false,
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

    fn watch_paths(&self, cwd: Option<PathBuf>) -> Vec<PathBuf> {
        match self {
            WatchMode::Vcs { .. } => cwd.into_iter().collect(),
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

fn visible_diffs(state: &AppState) -> std::borrow::Cow<'_, [FileDiff]> {
    match &state.filter {
        Some(pattern) => std::borrow::Cow::Owned(filter_file_diffs(state.file_diffs.clone(), pattern)),
        None => std::borrow::Cow::Borrowed(&state.file_diffs),
    }
}

fn run_watch_loop(mode: &WatchMode<'_>, cli: &Cli) -> Result<()> {
    let mut state = AppState::new(cli.changed_only, cli.full_context, cli.filter.clone());
    state.file_diffs = mode.compute_diffs_fast(&mut state, cli)?;

    let (tx, rx) = mpsc::channel();

    let fs_tx = tx.clone();
    let _debouncer = watch::setup_watcher(mode.watch_paths(std::env::current_dir().ok()), move || {
        let _ = fs_tx.send(AppEvent::FileChanged);
    })?;

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
        draw(&mut terminal, &mut state)?;
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

fn draw(terminal: &mut DefaultTerminal, state: &mut AppState) -> Result<()> {
    let diffs = visible_diffs(state);
    let opts = state.render_opts();
    let scroll = state.scroll;
    let mut result = render::RenderResult { section_offsets: vec![], max_scroll: 0 };
    terminal.draw(|frame| {
        result = render::render(frame, &diffs, scroll, opts);
    })?;
    drop(diffs);
    state.section_offsets = result.section_offsets;
    state.max_scroll = result.max_scroll;
    state.scroll = state.scroll.min(state.max_scroll);
    state.needs_redraw = false;
    Ok(())
}

fn run_event_loop(
    terminal: &mut DefaultTerminal,
    state: &mut AppState,
    rx: &mpsc::Receiver<AppEvent>,
    mode: &WatchMode<'_>,
    cli: &Cli,
) -> Result<()> {
    loop {
        if state.needs_redraw {
            draw(terminal, state)?;
        }

        if event::poll(Duration::from_millis(50))? {
            match state.apply(input::action_for(&event::read()?)) {
                Effect::Quit => return Ok(()),
                Effect::Repaint => terminal.clear()?,
                Effect::Redraw | Effect::None => {}
            }
        }

        match rx.try_recv() {
            Ok(AppEvent::FileChanged) => {
                state.cached_merge_base = None;
                state.file_diffs = mode.recompute(state, cli)?;
                state.needs_redraw = true;
            }
            Ok(AppEvent::Tick) => {
                if mode.merge_base_moved(state) {
                    state.cached_merge_base = None;
                    state.file_diffs = mode.recompute(state, cli)?;
                    state.needs_redraw = true;
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::StubVcs;

    fn stub_vcs() -> StubVcs {
        StubVcs {
            changed: vec![PathBuf::from("spec/models/user_spec.rb"), PathBuf::from("README.md")],
            ..StubVcs::default()
        }
    }

    fn vcs_mode(vcs: &StubVcs) -> WatchMode<'_> {
        WatchMode::Vcs { vcs, base_rev: "main".into(), head_rev: "HEAD".into() }
    }

    #[test]
    fn vcs_mode_sources_only_changed_test_files() {
        let vcs = stub_vcs();
        let files = vcs_mode(&vcs).with_source(Some("base"), |s| s.list_files()).expect("files");
        assert_eq!(files, vec!["spec/models/user_spec.rb".to_string()]);
    }

    #[test]
    fn merge_base_is_cached_then_recomputed_once_stale() {
        let vcs = stub_vcs();
        let mode = vcs_mode(&vcs);
        let mut state = AppState::new(false, false, None);
        assert_eq!(mode.resolve_merge_base(&mut state).as_deref(), Some("base"));
        state.cached_merge_base = Some("cached".into());
        assert_eq!(mode.resolve_merge_base(&mut state).as_deref(), Some("cached"));
        state.merge_base_time = Instant::now() - Duration::from_secs(3);
        assert_eq!(mode.resolve_merge_base(&mut state).as_deref(), Some("base"));
        assert_eq!(state.cached_merge_base.as_deref(), Some("base"));
    }

    #[test]
    fn a_tick_notices_only_a_moved_merge_base() {
        let vcs = stub_vcs();
        let mode = vcs_mode(&vcs);
        let mut state = AppState::new(false, false, None);
        state.cached_merge_base = Some("base".into());
        assert!(!mode.merge_base_moved(&state));
        state.cached_merge_base = Some("older".into());
        assert!(mode.merge_base_moved(&state));
        let dirs = WatchMode::Directory { base: "a".into(), head: "b".into() };
        assert!(!dirs.merge_base_moved(&state));
    }

    #[test]
    fn directory_mode_watches_both_sides_once() {
        let two = WatchMode::Directory { base: "base".into(), head: "head".into() };
        assert_eq!(two.watch_paths(None), vec![PathBuf::from("head"), PathBuf::from("base")]);
        let one = WatchMode::Directory { base: "same".into(), head: "same".into() };
        assert_eq!(one.watch_paths(None), vec![PathBuf::from("same")]);
    }

    #[test]
    fn vcs_mode_watches_the_working_directory() {
        let vcs = stub_vcs();
        let mode = vcs_mode(&vcs);
        assert_eq!(mode.watch_paths(Some(PathBuf::from("/repo"))), vec![PathBuf::from("/repo")]);
        assert!(mode.watch_paths(None).is_empty());
    }

    #[test]
    fn a_filter_narrows_what_is_drawn() {
        let file = |path: &str| FileDiff {
            path: path.into(),
            nodes: crate::diff::diff_spec_nodes(&[], &[crate::parse::SpecNode::spec("works", 1)]),
        };
        let mut state = AppState::new(false, false, None);
        state.file_diffs = vec![file("models::user"), file("models::post")];
        assert_eq!(visible_diffs(&state).len(), 2);
        state.filter = Some("USER".into());
        let visible = visible_diffs(&state);
        assert_eq!(visible.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), vec!["models::user"]);
    }

    const SHARED: &str = "RSpec.shared_examples \"auditable\" do\n  it \"records the actor\" do\n  end\nend\n";
    const USES_SHARED: &str =
        "RSpec.describe User do\n  it_behaves_like \"auditable\"\n  it \"works\" do\n  end\nend\n";

    struct Dirs {
        _root: tempfile::TempDir,
        base: PathBuf,
        head: PathBuf,
    }

    fn dirs(head_user: &str) -> Dirs {
        let root = tempfile::TempDir::new().expect("tempdir");
        let base = root.path().join("base");
        let head = root.path().join("head");
        for side in [&base, &head] {
            std::fs::create_dir_all(side.join("spec/models")).expect("mkdir");
            std::fs::create_dir_all(side.join("spec/support")).expect("mkdir");
            std::fs::write(side.join("spec/support/auditable.rb"), SHARED).expect("shared");
        }
        std::fs::write(head.join("spec/models/user_spec.rb"), head_user).expect("head");
        Dirs { _root: root, base, head }
    }

    fn dir_mode(d: &Dirs) -> WatchMode<'static> {
        WatchMode::Directory { base: d.base.clone(), head: d.head.clone() }
    }

    fn cli() -> Cli {
        <Cli as clap::Parser>::parse_from(["specdiff"])
    }

    fn outline(diffs: &[FileDiff]) -> Vec<String> {
        fn walk(nodes: &[crate::diff::types::DiffNode], out: &mut Vec<String>) {
            for n in nodes {
                out.push(n.name.clone());
                walk(&n.children, out);
            }
        }
        let mut out = Vec::new();
        for d in diffs {
            walk(&d.nodes, &mut out);
        }
        out
    }

    #[test]
    fn a_shared_example_inclusion_needs_the_registry_and_a_plain_spec_does_not() {
        let mut state = AppState::new(false, false, None);
        let shared = dirs(USES_SHARED);
        assert!(dir_mode(&shared).needs_shared_scan(&mut state, &cli()));
        let plain = dirs("RSpec.describe User do\n  it \"works\" do\n  end\nend\n");
        assert!(!dir_mode(&plain).needs_shared_scan(&mut state, &cli()));
    }

    #[test]
    fn the_registry_expands_an_inclusion_the_fast_path_leaves_as_a_placeholder() {
        let d = dirs(USES_SHARED);
        let mode = dir_mode(&d);
        let mut state = AppState::new(false, false, None);
        let registry = mode.build_registry(&mut state, &cli());
        assert!(registry.get("auditable").is_some());

        let resolved = outline(&mode.compute_diffs_with_registry(&mut state, &cli(), &registry).expect("diffs"));
        assert!(resolved.contains(&"records the actor".to_string()), "{resolved:?}");
        let fast = outline(&mode.compute_diffs_fast(&mut state, &cli()).expect("diffs"));
        assert!(!fast.contains(&"records the actor".to_string()), "{fast:?}");

        assert_eq!(outline(&mode.recompute(&mut state, &cli()).expect("fast")), fast);
        state.shared_registry = Some(registry);
        assert_eq!(outline(&mode.recompute(&mut state, &cli()).expect("resolved")), resolved);
    }
}
