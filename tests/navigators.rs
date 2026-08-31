//! L3: real yazi, real broot, a real pty. Press Enter on a file in the file manager and
//! prove the chain — navigator config → navigator → `smartopen "$@"` → matcher →
//! runner → a file on disk — end to end, with the real built binary and no shims.
//!
//! Gated behind SMARTOPEN_NAVIGATOR_TESTS=1 so a plain `cargo test` needs neither tool;
//! CI always sets it. `portable-pty` does POSIX ptys and Windows ConPTY through one API,
//! which is what makes this layer cross-platform.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const SMARTOPEN: &str = env!("CARGO_BIN_EXE_smartopen");
const OPN: &str = env!("CARGO_BIN_EXE_opn");

fn enabled() -> bool {
    std::env::var("SMARTOPEN_NAVIGATOR_TESTS").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn on_path(program: &str) -> bool {
    StdCommand::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A sandboxed home with a config whose only `.csv` command records the path it was
/// given, so a single match runs without a menu and leaves evidence.
struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let root = tempfile::tempdir().unwrap();
        let sandbox = Sandbox { root };
        fs::create_dir_all(sandbox.config_dir().join("smartopen")).unwrap();
        fs::write(
            sandbox.config_dir().join("smartopen").join("config.toml"),
            "[[extension]]\nextensions = [\"csv\"]\n\n[[extension.command]]\nlabel = \"Record\"\nrun = \"printf %s {path} > \\\"$SMARTOPEN_TEST_OUT\\\"\"\n",
        )
        .unwrap();
        fs::create_dir_all(sandbox.files()).unwrap();
        fs::write(sandbox.files().join("sample.csv"), "a,b\n1,2\n").unwrap();
        // A stand-in for `$TERMINAL`: drops `-e` and runs the command in place, so the
        // broot verb's "open in a new terminal window" happens in this pty.
        let bin = sandbox.bin_dir();
        fs::create_dir_all(&bin).unwrap();
        let faketerm = bin.join("faketerm");
        fs::write(
            &faketerm,
            "#!/bin/sh\n[ \"$1\" = -e ] && shift\nexec \"$@\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&faketerm, fs::Permissions::from_mode(0o755)).unwrap();
        sandbox
    }

    fn path(&self) -> &Path {
        self.root.path()
    }
    fn config_dir(&self) -> PathBuf {
        self.path().join("config")
    }
    fn files(&self) -> PathBuf {
        self.path().join("files")
    }
    fn bin_dir(&self) -> PathBuf {
        self.path().join("bin")
    }
    fn out_file(&self) -> PathBuf {
        self.path().join("recorded.txt")
    }

    /// `PATH` with the built binaries first, so `smartopen`/`opn` inside the navigator
    /// are the ones under test.
    fn path_env(&self) -> String {
        let bins = Path::new(SMARTOPEN).parent().unwrap();
        format!(
            "{}:{}:{}",
            bins.display(),
            self.bin_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn env(&self, cmd: &mut CommandBuilder) {
        cmd.env_clear();
        cmd.env("PATH", self.path_env());
        cmd.env("HOME", self.path());
        cmd.env("XDG_CONFIG_HOME", self.config_dir());
        cmd.env("XDG_STATE_HOME", self.path().join("state"));
        cmd.env("XDG_DATA_HOME", self.path().join("data"));
        cmd.env("XDG_CACHE_HOME", self.path().join("cache"));
        // broot does not follow XDG on macOS; this is the override it honours everywhere.
        cmd.env("BROOT_CONFIG_DIR", self.config_dir().join("broot"));
        cmd.env("TERM", "xterm-256color");
        cmd.env("TERMINAL", self.bin_dir().join("faketerm"));
        cmd.env("SMARTOPEN_TEST_OUT", self.out_file());
        cmd.env("SMARTOPEN_NO_HISTORY", "1");
        cmd.env("LANG", "C.UTF-8");
    }

    fn run_smartopen(&self, bin: &str, args: &[&str]) {
        let status = StdCommand::new(bin)
            .args(args)
            .env_clear()
            .env("PATH", self.path_env())
            .env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.config_dir())
            .env("XDG_STATE_HOME", self.path().join("state"))
            .status()
            .unwrap();
        assert!(status.success(), "{bin} {args:?} failed");
    }

    fn wait_for_recording(&self, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(text) = fs::read_to_string(self.out_file())
                && !text.is_empty()
            {
                return Some(text);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

/// A child in a pty with its output captured on a thread.
struct Pty {
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Pty {
    fn spawn(cmd: CommandBuilder) -> Pty {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&output);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        // Keep the master alive for the child's lifetime by leaking it into the struct
        // through the writer/reader clones; the pair itself may drop.
        std::mem::forget(pair.master);
        Pty {
            writer,
            output,
            child,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    /// Wait until the captured output satisfies `pred`, or the timeout passes.
    fn wait_for(&self, pred: impl Fn(&str) -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if pred(&self.output()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn skip_unless(tool: &str) -> bool {
    if !enabled() {
        eprintln!("skipped: set SMARTOPEN_NAVIGATOR_TESTS=1 to run the navigator tests");
        return true;
    }
    if !on_path(tool) {
        panic!("SMARTOPEN_NAVIGATOR_TESTS is set but {tool} is not on PATH");
    }
    false
}

fn yazi_enter_reaches(bin: &str) {
    let sandbox = Sandbox::new();
    let yazi_toml = sandbox.config_dir().join("yazi").join("yazi.toml");
    sandbox.run_smartopen(
        bin,
        &["yazi", "apply", "--target", yazi_toml.to_str().unwrap()],
    );
    let written = fs::read_to_string(&yazi_toml).unwrap();
    let delegate = Path::new(bin).file_name().unwrap().to_str().unwrap();
    assert!(
        written.contains(&format!("run = '{delegate} %s'")),
        "yazi.toml must delegate to {delegate}:\n{written}"
    );

    let mut cmd = CommandBuilder::new("yazi");
    cmd.arg(sandbox.files());
    sandbox.env(&mut cmd);
    let mut pty = Pty::spawn(cmd);

    // yazi asks the terminal what it is (DA1, `ESC [ 0 c`) and waits for the answer
    // before it reads keys; a bare pty never answers, so reply the way a VT does.
    assert!(
        pty.wait_for(|out| out.contains("\x1b[0c"), Duration::from_secs(10)),
        "yazi never sent DA1:\n{}",
        pty.output()
    );
    pty.send(b"\x1b[?62;22c");

    // yazi draws the single file, cursor on it; Enter is `open`.
    assert!(
        pty.wait_for(|out| out.contains("sample.csv"), Duration::from_secs(10)),
        "yazi never drew the file list:\n{}",
        pty.output()
    );
    std::thread::sleep(Duration::from_millis(500));
    pty.send(b"\r");

    let recorded = sandbox.wait_for_recording(Duration::from_secs(10));
    pty.send(b"q");
    pty.kill();

    let recorded = recorded
        .unwrap_or_else(|| panic!("Enter in yazi never reached {delegate}:\n{}", pty.output()));
    assert_eq!(
        Path::new(&recorded).canonicalize().unwrap(),
        sandbox.files().join("sample.csv").canonicalize().unwrap()
    );
}

#[test]
fn enter_in_yazi_runs_the_matched_command_through_smartopen() {
    if skip_unless("yazi") {
        return;
    }
    yazi_enter_reaches(SMARTOPEN);
}

#[test]
fn enter_in_yazi_works_through_the_opn_alias_too() {
    if skip_unless("yazi") {
        return;
    }
    yazi_enter_reaches(OPN);
}

#[test]
fn yazi_refuses_a_malformed_opener_so_the_positive_test_means_something() {
    if skip_unless("yazi") {
        return;
    }
    let sandbox = Sandbox::new();
    let yazi_dir = sandbox.config_dir().join("yazi");
    fs::create_dir_all(&yazi_dir).unwrap();
    fs::write(
        yazi_dir.join("yazi.toml"),
        "[opener]\nthis is = not = toml\n",
    )
    .unwrap();

    let mut cmd = CommandBuilder::new("yazi");
    cmd.arg(sandbox.files());
    sandbox.env(&mut cmd);
    let mut pty = Pty::spawn(cmd);
    let complained = pty.wait_for(
        |out| {
            let lower = out.to_lowercase();
            lower.contains("error") || lower.contains("failed") || lower.contains("parse")
        },
        Duration::from_secs(10),
    );
    pty.kill();
    assert!(
        complained,
        "yazi accepted a broken yazi.toml:\n{}",
        pty.output()
    );
}

#[test]
fn enter_in_broot_runs_the_matched_command_through_smartopen() {
    if skip_unless("broot") {
        return;
    }
    let sandbox = Sandbox::new();
    let broot_dir = sandbox.config_dir().join("broot");
    sandbox.run_smartopen(
        SMARTOPEN,
        &["broot", "apply", "--target", broot_dir.to_str().unwrap()],
    );
    // Without this, a fresh broot asks about installing its shell function first.
    let status = StdCommand::new("broot")
        .args(["--set-install-state", "refused"])
        .env("BROOT_CONFIG_DIR", &broot_dir)
        .env("XDG_CONFIG_HOME", sandbox.config_dir())
        .env("XDG_DATA_HOME", sandbox.path().join("data"))
        .env("HOME", sandbox.path())
        .status()
        .unwrap();
    assert!(status.success());

    let mut cmd = CommandBuilder::new("broot");
    cmd.arg(sandbox.files());
    sandbox.env(&mut cmd);
    let mut pty = Pty::spawn(cmd);

    assert!(
        pty.wait_for(|out| out.contains("sample.csv"), Duration::from_secs(10)),
        "broot never drew the tree:\n{}",
        pty.output()
    );
    // Down onto the file (the root line is selected first), then Enter for our verb.
    pty.send(b"\x1b[B");
    std::thread::sleep(Duration::from_millis(300));
    pty.send(b"\r");

    let recorded = sandbox.wait_for_recording(Duration::from_secs(10));
    pty.send(b"\x1b\x1b:q\r");
    pty.kill();

    let recorded = recorded
        .unwrap_or_else(|| panic!("Enter in broot never reached smartopen:\n{}", pty.output()));
    assert_eq!(
        Path::new(&recorded).canonicalize().unwrap(),
        sandbox.files().join("sample.csv").canonicalize().unwrap()
    );
}

#[test]
fn the_zsh_widget_puts_the_pick_on_the_prompt_line_unexecuted() {
    if skip_unless("zsh") {
        return;
    }
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.config_dir().join("smartopen").join("config.toml"),
        "[[shortcut]]\nlabel = \"Marker\"\nrun = \"echo WIDGET_RAN_MARKER\"\n",
    )
    .unwrap();
    let zdotdir = sandbox.path().join("zdot");
    fs::create_dir_all(&zdotdir).unwrap();
    let snippet = StdCommand::new(SMARTOPEN)
        .args(["shell", "zsh"])
        .output()
        .unwrap()
        .stdout;
    fs::write(
        zdotdir.join(".zshrc"),
        format!("PS1='PROMPT> '\n{}\n", String::from_utf8(snippet).unwrap()),
    )
    .unwrap();

    let mut cmd = CommandBuilder::new("zsh");
    // -d skips /etc/zsh/*: Ubuntu's global rc runs compinit, which stops at an
    // "insecure directories" prompt on the runner. Only the sandbox .zshrc is wanted.
    cmd.args(["-d", "-i"]);
    sandbox.env(&mut cmd);
    cmd.env("ZDOTDIR", &zdotdir);
    let mut pty = Pty::spawn(cmd);

    assert!(
        pty.wait_for(|out| out.contains("PROMPT>"), Duration::from_secs(10)),
        "{}",
        pty.output()
    );
    pty.send(b"\x07"); // Ctrl-G
    // The picker draws on the tty; the single shortcut is row 1. Enter picks it.
    assert!(
        pty.wait_for(|out| out.contains("Marker"), Duration::from_secs(10)),
        "the picker never appeared:\n{}",
        pty.output()
    );
    pty.send(b"\r");
    assert!(
        pty.wait_for(
            |out| out.contains("PROMPT> echo WIDGET_RAN_MARKER"),
            Duration::from_secs(10)
        ),
        "the command never landed on the prompt line:\n{}",
        pty.output()
    );
    std::thread::sleep(Duration::from_millis(300));
    let output = pty.output();
    pty.kill();
    assert!(
        !output
            .lines()
            .any(|line| line.trim() == "WIDGET_RAN_MARKER"),
        "the widget must paste, not run:\n{output}"
    );
}
