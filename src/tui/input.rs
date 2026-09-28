use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

pub const PAGE: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextSection,
    PrevSection,
    ScrollDown(usize),
    ScrollUp(usize),
    Top,
    Bottom,
    ToggleChangedOnly,
    Repaint,
    Ignore,
}

pub fn action_for(event: &Event) -> Action {
    match event {
        Event::Key(key) => action_for_key(*key),
        Event::Resize(..) => Action::Repaint,
        _ => Action::Ignore,
    }
}

fn action_for_key(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('c') => Action::ToggleChangedOnly,
        KeyCode::Char('j') => Action::NextSection,
        KeyCode::Char('k') => Action::PrevSection,
        KeyCode::Down => Action::ScrollDown(1),
        KeyCode::Up => Action::ScrollUp(1),
        KeyCode::PageDown => Action::ScrollDown(PAGE),
        KeyCode::PageUp => Action::ScrollUp(PAGE),
        KeyCode::Home | KeyCode::Char('g') => Action::Top,
        KeyCode::End | KeyCode::Char('G') => Action::Bottom,
        _ => Action::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    #[test]
    fn q_escape_and_ctrl_c_quit() {
        assert_eq!(action_for(&key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(action_for(&key(KeyCode::Esc)), Action::Quit);
        assert_eq!(action_for(&ctrl('c')), Action::Quit);
    }

    #[test]
    fn plain_c_toggles_changed_only_rather_than_quitting() {
        assert_eq!(action_for(&key(KeyCode::Char('c'))), Action::ToggleChangedOnly);
    }

    #[test]
    fn every_binding_maps_to_its_action() {
        let bindings = [
            (KeyCode::Char('j'), Action::NextSection),
            (KeyCode::Char('k'), Action::PrevSection),
            (KeyCode::Down, Action::ScrollDown(1)),
            (KeyCode::Up, Action::ScrollUp(1)),
            (KeyCode::PageDown, Action::ScrollDown(20)),
            (KeyCode::PageUp, Action::ScrollUp(20)),
            (KeyCode::Home, Action::Top),
            (KeyCode::Char('g'), Action::Top),
            (KeyCode::End, Action::Bottom),
            (KeyCode::Char('G'), Action::Bottom),
        ];
        for (code, action) in bindings {
            assert_eq!(action_for(&key(code)), action, "{code:?}");
        }
    }

    #[test]
    fn unbound_keys_and_other_events_are_ignored() {
        assert_eq!(action_for(&key(KeyCode::Char('x'))), Action::Ignore);
        assert_eq!(action_for(&ctrl('x')), Action::Ignore);
        assert_eq!(action_for(&Event::FocusGained), Action::Ignore);
        assert_eq!(action_for(&Event::Paste("q".into())), Action::Ignore);
    }

    #[test]
    fn a_resize_forces_a_repaint() {
        assert_eq!(action_for(&Event::Resize(80, 24)), Action::Repaint);
    }
}
