use super::input::Action;
use super::render::RenderOptions;
use crate::diff::types::FileDiff;
use crate::parse::shared::SharedExampleRegistry;
use std::time::{Duration, Instant};

pub const MERGE_BASE_TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    Redraw,
    Repaint,
}

#[allow(clippy::struct_excessive_bools)]
pub struct AppState {
    pub file_diffs: Vec<FileDiff>,
    pub scroll: usize,
    pub section_offsets: Vec<usize>,
    pub changed_only: bool,
    pub full_context: bool,
    pub filter: Option<String>,
    pub needs_redraw: bool,
    pub max_scroll: usize,
    pub cached_merge_base: Option<String>,
    pub merge_base_time: Instant,
    pub shared_registry: Option<SharedExampleRegistry>,
}

impl AppState {
    pub fn new(changed_only: bool, full_context: bool, filter: Option<String>) -> Self {
        Self {
            file_diffs: vec![],
            scroll: 0,
            section_offsets: vec![],
            changed_only,
            full_context,
            filter,
            needs_redraw: true,
            max_scroll: 0,
            cached_merge_base: None,
            merge_base_time: Instant::now(),
            shared_registry: None,
        }
    }

    pub fn render_opts(&self) -> RenderOptions {
        RenderOptions { changed_only: self.changed_only, full_context: self.full_context }
    }

    pub fn apply(&mut self, action: Action) -> Effect {
        match action {
            Action::Quit => return Effect::Quit,
            Action::Ignore => return Effect::None,
            Action::Repaint => {
                self.needs_redraw = true;
                return Effect::Repaint;
            }
            Action::NextSection => self.scroll = next_section(self.scroll, &self.section_offsets),
            Action::PrevSection => self.scroll = prev_section(self.scroll, &self.section_offsets),
            Action::ScrollDown(n) => self.scroll = self.scroll.saturating_add(n),
            Action::ScrollUp(n) => self.scroll = self.scroll.saturating_sub(n),
            Action::Top => self.scroll = 0,
            Action::Bottom => self.scroll = usize::MAX,
            Action::ToggleChangedOnly => self.changed_only = !self.changed_only,
        }
        self.scroll = self.scroll.min(self.max_scroll);
        self.needs_redraw = true;
        Effect::Redraw
    }

    pub fn cached_merge_base(&self, now: Instant) -> Option<&String> {
        if merge_base_is_stale(now.saturating_duration_since(self.merge_base_time)) {
            return None;
        }
        self.cached_merge_base.as_ref()
    }

    pub fn remember_merge_base(&mut self, merge_base: String, now: Instant) {
        self.cached_merge_base = Some(merge_base);
        self.merge_base_time = now;
    }

    pub fn merge_base_moved(&self, fresh: &str) -> bool {
        self.cached_merge_base.as_deref() != Some(fresh)
    }
}

pub fn merge_base_is_stale(age: Duration) -> bool {
    age > MERGE_BASE_TTL
}

pub fn next_section(current: usize, offsets: &[usize]) -> usize {
    offsets.iter().find(|&&o| o > current).copied().unwrap_or(current)
}

pub fn prev_section(current: usize, offsets: &[usize]) -> usize {
    offsets.iter().rev().find(|&&o| o < current).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrollable() -> AppState {
        let mut state = AppState::new(false, false, None);
        state.section_offsets = vec![0, 10, 20];
        state.max_scroll = 30;
        state.needs_redraw = false;
        state
    }

    #[test]
    fn quit_and_ignore_change_nothing() {
        let mut state = scrollable();
        state.scroll = 5;
        assert_eq!(state.apply(Action::Quit), Effect::Quit);
        assert_eq!(state.apply(Action::Ignore), Effect::None);
        assert_eq!(state.scroll, 5);
        assert!(!state.needs_redraw);
    }

    #[test]
    fn repaint_redraws_the_whole_frame() {
        let mut state = scrollable();
        assert_eq!(state.apply(Action::Repaint), Effect::Repaint);
        assert!(state.needs_redraw);
    }

    #[test]
    fn scrolling_moves_and_clamps_to_the_last_line() {
        let mut state = scrollable();
        assert_eq!(state.apply(Action::ScrollDown(1)), Effect::Redraw);
        assert!(state.needs_redraw);
        assert_eq!(state.scroll, 1);
        state.apply(Action::ScrollDown(20));
        assert_eq!(state.scroll, 21);
        state.apply(Action::ScrollDown(20));
        assert_eq!(state.scroll, 30);
        state.apply(Action::ScrollUp(1));
        assert_eq!(state.scroll, 29);
        state.apply(Action::ScrollUp(100));
        assert_eq!(state.scroll, 0);
        state.apply(Action::Bottom);
        assert_eq!(state.scroll, 30);
        state.apply(Action::Top);
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn section_jumps_follow_the_file_offsets() {
        let mut state = scrollable();
        state.apply(Action::NextSection);
        assert_eq!(state.scroll, 10);
        state.apply(Action::NextSection);
        assert_eq!(state.scroll, 20);
        state.apply(Action::PrevSection);
        assert_eq!(state.scroll, 10);
    }

    #[test]
    fn c_toggles_changed_only_both_ways() {
        let mut state = scrollable();
        state.apply(Action::ToggleChangedOnly);
        assert!(state.changed_only);
        assert!(state.render_opts().changed_only);
        state.apply(Action::ToggleChangedOnly);
        assert!(!state.changed_only);
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

    #[test]
    fn merge_base_goes_stale_after_two_seconds_not_at_them() {
        assert!(!merge_base_is_stale(Duration::from_millis(1999)));
        assert!(!merge_base_is_stale(Duration::from_secs(2)));
        assert!(merge_base_is_stale(Duration::from_millis(2001)));
    }

    #[test]
    fn a_cached_merge_base_is_served_until_it_is_stale() {
        let mut state = AppState::new(false, false, None);
        let then = Instant::now();
        assert_eq!(state.cached_merge_base(then), None);
        state.remember_merge_base("abc123".into(), then);
        assert_eq!(state.cached_merge_base(then + Duration::from_secs(1)).map(String::as_str), Some("abc123"));
        assert_eq!(state.cached_merge_base(then + Duration::from_secs(3)), None);
    }

    #[test]
    fn merge_base_moved_compares_with_the_cached_one() {
        let mut state = AppState::new(false, false, None);
        assert!(state.merge_base_moved("abc123"));
        state.remember_merge_base("abc123".into(), Instant::now());
        assert!(!state.merge_base_moved("abc123"));
        assert!(state.merge_base_moved("def456"));
    }
}
