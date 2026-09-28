//! The real binary in a pseudo-terminal, for what crosses the terminal: quitting and
//! restoring it, resizing, and output that differs between a terminal and a pipe. Every key
//! and view is also unit-tested in `src/tui`; `tests/tui.toml` maps each to its test.

#[path = "support/pty.rs"]
mod pty;

const USER_BASE: &str = "RSpec.describe User do\n  it \"validates email\" do\n  end\nend\n";
const USER_HEAD: &str =
    "RSpec.describe User do\n  it \"validates email\" do\n  end\n  it \"validates uniqueness\" do\n  end\nend\n";

fn world() -> pty::World {
    let world = pty::World::new();
    world.write("base", "spec/models/user_spec.rb", USER_BASE);
    world.write("head", "spec/models/user_spec.rb", USER_HEAD);
    world
}

fn open(world: &pty::World) -> pty::Tui {
    let mut tui = world.tui(&[]);
    tui.wait_for_text("validates uniqueness");
    tui
}

#[test]
fn q_quits_and_restores_the_terminal() {
    let world = world();
    let mut tui = open(&world);
    assert!(tui.emitted(b"\x1b[?1049h"), "the TUI should enter the alternate screen");
    tui.press("q");
    assert!(tui.wait_for_exit(), "q should exit successfully");
    assert!(tui.emitted(b"\x1b[?1049l"), "quitting should leave the alternate screen");
    world.assert_nothing_ran();
}

#[test]
fn ctrl_c_quits() {
    let world = world();
    let mut tui = open(&world);
    tui.press("\x03");
    assert!(tui.wait_for_exit(), "Ctrl-C should exit successfully");
    assert!(tui.emitted(b"\x1b[?1049l"));
}

#[test]
fn escape_quits() {
    let world = world();
    let mut tui = open(&world);
    tui.press("\x1b");
    assert!(tui.wait_for_exit(), "Esc should exit successfully");
}

#[test]
fn a_resize_repaints_at_the_new_size() {
    let world = world();
    let mut tui = open(&world);
    let footer = "[q]uit  [c]hanged-only  [j/k] next/prev file";
    tui.wait_for("the footer on the last row", |t| t.row(pty::ROWS - 1) == footer);
    tui.resize(12, 60);
    tui.wait_for("the footer on the new last row", |t| t.row(11) == footer);
    assert!(tui.screen().contains("validates uniqueness"));
    tui.press("q");
    assert!(tui.wait_for_exit());
}

#[test]
fn print_is_coloured_on_a_terminal_and_plain_in_a_pipe() {
    let world = world();
    let mut tui = world.tui(&["--print"]);
    assert!(tui.wait_for_exit());
    assert!(tui.emitted(b"\x1b[32m+ "), "an added spec should be green on a terminal");
    assert!(tui.screen().contains("+      validates uniqueness"));

    let piped = world.piped(&["--print"]);
    assert!(piped.status.success());
    let stdout = String::from_utf8(piped.stdout).expect("utf8");
    assert!(!stdout.contains('\x1b'), "piped output should carry no escapes: {stdout:?}");
    assert!(stdout.contains("+      validates uniqueness"));
}

#[test]
fn interactive_mode_prints_when_stdout_is_piped() {
    let world = world();
    let piped = world.piped(&[]);
    assert!(piped.status.success(), "stderr: {}", String::from_utf8_lossy(&piped.stderr));
    let stdout = String::from_utf8(piped.stdout).expect("utf8");
    assert!(!stdout.contains('\x1b'), "no terminal setup in a pipe: {stdout:?}");
    assert_eq!(stdout.lines().next(), Some("specdiff  +1 "));
    assert!(stdout.contains("+      validates uniqueness"));
    world.assert_nothing_ran();
}
