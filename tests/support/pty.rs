//! specdiff in a pseudo-terminal, for tests of what crosses the terminal.
//!
//! Modelled on purview's `tests/support/pty.rs` (isolation, stubs) and branchdiff's
//! `tests/integration/harness/session.rs` (reading escape sequences). The binary is the one
//! cargo just built (`CARGO_BIN_EXE_specdiff`), the environment is cleared, HOME is a temp
//! dir and PATH is the stub directory, so nothing on the developer's machine is reached.
//! Everything the TUI decides on its own is a unit or render test in `src/tui`.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

pub const ROWS: u16 = 30;
pub const COLS: u16 = 100;
const TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(20);

pub fn programs() -> Vec<String> {
    let manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/tui.toml")).expect("tui.toml"))
            .expect("tui.toml parses");
    manifest["programs"]
        .as_array()
        .expect("programs")
        .iter()
        .map(|p| p.as_str().expect("a program name").to_string())
        .collect()
}

pub struct World {
    root: tempfile::TempDir,
    pub stubs: Stubs,
}

impl World {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("a world");
        for dir in ["home", "base/spec/models", "head/spec/models"] {
            std::fs::create_dir_all(root.path().join(dir)).expect("a dir");
        }
        let names = programs();
        let stubs = Stubs::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
        Self { root, stubs }
    }

    pub fn path(&self) -> PathBuf {
        self.root.path().canonicalize().expect("canonical")
    }

    pub fn home(&self) -> String {
        self.path().join("home").display().to_string()
    }

    pub fn write(&self, side: &str, rel: &str, content: &str) {
        std::fs::write(self.path().join(side).join(rel), content).expect("a spec file");
    }

    pub fn dir_args(&self) -> Vec<String> {
        vec![
            "--base-dir".into(),
            self.path().join("base").display().to_string(),
            "--head-dir".into(),
            self.path().join("head").display().to_string(),
        ]
    }

    pub fn env(&self) -> Vec<(String, String)> {
        vec![
            ("TERM".into(), "xterm-256color".into()),
            ("HOME".into(), self.home()),
            ("PATH".into(), self.stubs.path()),
        ]
    }

    pub fn tui(&self, extra: &[&str]) -> Tui {
        let mut args = self.dir_args();
        args.extend(extra.iter().map(|a| (*a).to_string()));
        Tui::spawn(&args, &self.env())
    }

    pub fn piped(&self, extra: &[&str]) -> std::process::Output {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_specdiff"));
        cmd.env_clear();
        cmd.args(self.dir_args()).args(extra);
        cmd.envs(self.env());
        cmd.stdin(std::process::Stdio::null());
        cmd.output().expect("specdiff runs")
    }

    pub fn assert_nothing_ran(&self) {
        for name in programs() {
            assert!(!self.stubs.was_called(&name), "{name} ran: {}", self.stubs.record(&name));
        }
    }
}

pub struct Tui {
    parser: vt100::Parser,
    raw: Vec<u8>,
    output: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Tui {
    pub fn spawn(args: &[String], env: &[(String, String)]) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize { rows: ROWS, cols: COLS, pixel_width: 0, pixel_height: 0 })
            .expect("a pty");
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_specdiff"));
        cmd.env_clear();
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd.args(args);
        let child = pair.slave.spawn_command(cmd).expect("specdiff starts");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("a reader");
        let writer = pair.master.take_writer().expect("a writer");
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        Self {
            parser: vt100::Parser::new(ROWS, COLS, 0),
            raw: Vec::new(),
            output,
            writer,
            master: pair.master,
            child,
        }
    }

    fn drain(&mut self) {
        while let Ok(bytes) = self.output.try_recv() {
            self.parser.process(&bytes);
            self.raw.extend_from_slice(&bytes);
        }
    }

    pub fn screen(&mut self) -> String {
        self.drain();
        self.parser.screen().contents()
    }

    pub fn row(&mut self, row: u16) -> String {
        self.drain();
        let (_, cols) = self.parser.screen().size();
        self.parser.screen().contents_between(row, 0, row, cols).trim_end().to_string()
    }

    pub fn emitted(&mut self, bytes: &[u8]) -> bool {
        self.drain();
        self.raw.windows(bytes.len()).any(|w| w == bytes)
    }

    pub fn press(&mut self, keys: &str) {
        self.writer.write_all(keys.as_bytes()).expect("a keypress");
        self.writer.flush().expect("a flush");
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.drain();
        self.master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .expect("a resize");
        self.parser.set_size(rows, cols);
    }

    pub fn wait_for(&mut self, what: &str, done: impl Fn(&mut Self) -> bool) {
        let start = Instant::now();
        loop {
            if done(self) {
                return;
            }
            let screen = self.screen();
            assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}. Screen:\n{screen}");
            std::thread::sleep(POLL);
        }
    }

    pub fn wait_for_text(&mut self, text: &str) {
        self.wait_for(text, |tui| tui.screen().contains(text));
    }

    pub fn wait_for_exit(&mut self) -> bool {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("a status") {
                self.drain();
                return status.success();
            }
            let screen = self.screen();
            assert!(start.elapsed() < TIMEOUT, "specdiff did not exit. Screen:\n{screen}");
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

pub struct Stubs {
    dir: tempfile::TempDir,
}

impl Stubs {
    pub fn new(names: &[&str]) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("a stub dir");
        for name in names {
            let record = dir.path().join(format!("{name}.called"));
            let script = format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{r}.tmp'\n\
                 if [ ! -t 0 ]; then cat >> '{r}.tmp'; fi\nmv '{r}.tmp' '{r}'\n",
                r = record.display(),
            );
            let path = dir.path().join(name);
            std::fs::write(&path, script).expect("a stub");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("executable");
        }
        Self { dir }
    }

    pub fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.dir.path().display())
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    pub fn was_called(&self, name: &str) -> bool {
        self.dir.path().join(format!("{name}.called")).exists()
    }

    pub fn record(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(format!("{name}.called"))).unwrap_or_default()
    }
}
