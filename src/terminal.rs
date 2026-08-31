//! `terminal = true`: run a command in a new terminal window, whatever the OS.
//!
//! The sample config used to hand-roll `${TERMINAL:-ghostty} --working-directory {path}`,
//! which is Linux-only and only right for ghostty. Every terminal spells "start in this
//! directory and run this" differently, so the spelling lives here, keyed by the
//! program's name: `$TERMINAL` if set, otherwise the first known terminal on `PATH`.
//! macOS goes through Terminal.app via osascript; Windows through Windows Terminal, or
//! `start cmd` without it.

use std::path::Path;

use anyhow::{Result, bail};

use crate::platform::Host;
use crate::runner::find_executable;
use crate::shell::Shell;

/// Linux terminals tried in order when `$TERMINAL` is unset.
const LINUX_CANDIDATES: &[&str] = &["ghostty", "foot", "kitty", "alacritty", "wezterm", "xterm"];

/// A command line that opens a new terminal in `cwd` running `command`.
pub fn wrap(command: &str, cwd: Option<&Path>) -> Result<String> {
    let host = Host::current();
    let program = match host {
        Host::Windows => std::env::var("TERMINAL")
            .ok()
            .or_else(|| find_executable("wt").map(|_| "wt".to_string()))
            .unwrap_or_else(|| "cmd".to_string()),
        Host::Macos => std::env::var("TERMINAL")
            .ok()
            .unwrap_or_else(|| "Terminal".to_string()),
        _ => match std::env::var("TERMINAL") {
            Ok(terminal) if !terminal.is_empty() => terminal,
            _ => LINUX_CANDIDATES
                .iter()
                .find(|name| find_executable(name).is_some())
                .map(|name| name.to_string())
                .unwrap_or_default(),
        },
    };
    wrap_for(host, &program, command, cwd)
}

/// The pure part: how `program` is told to open in `cwd` and run `command`.
pub fn wrap_for(host: Host, program: &str, command: &str, cwd: Option<&Path>) -> Result<String> {
    let shell = Shell::current();
    let cwd_str = cwd.map(|p| p.display().to_string());

    match host {
        Host::Windows => {
            let dir = cwd_str.as_deref().map(|d| shell.quote(d)).transpose()?;
            let name = Path::new(program)
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            Ok(if name == "wt" {
                match dir {
                    Some(dir) => format!("wt -d {dir} cmd /k {command}"),
                    None => format!("wt cmd /k {command}"),
                }
            } else {
                match dir {
                    Some(dir) => format!("start \"\" cmd /k \"cd /d {dir} && {command}\""),
                    None => format!("start \"\" cmd /k \"{command}\""),
                }
            })
        }
        Host::Macos => {
            // Terminal.app (or iTerm) runs the line in a fresh shell via AppleScript.
            let script = match &cwd_str {
                Some(dir) => format!("cd {} && {command}", Shell::Posix.quote(dir)?),
                None => command.to_string(),
            };
            let app = if program.is_empty() {
                "Terminal"
            } else {
                program
            };
            Ok(format!(
                "osascript -e {}",
                Shell::Posix.quote(&format!(
                    "tell application \"{}\" to do script \"{}\"",
                    applescript_escape(app),
                    applescript_escape(&script)
                ))?
            ))
        }
        Host::Linux | Host::OtherUnix => {
            if program.is_empty() {
                bail!(
                    "no terminal found: set $TERMINAL, or install one of {}",
                    LINUX_CANDIDATES.join(", ")
                );
            }
            let run = format!("sh -c {}", Shell::Posix.quote(command)?);
            let name = Path::new(program)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| program.to_string());
            let dir = cwd_str
                .as_deref()
                .map(|d| Shell::Posix.quote(d))
                .transpose()?;
            Ok(match (name.as_str(), dir) {
                ("ghostty", Some(dir)) => format!("{program} --working-directory={dir} -e {run}"),
                ("foot", Some(dir)) => format!("{program} --working-directory={dir} {run}"),
                ("kitty", Some(dir)) => format!("{program} --directory {dir} {run}"),
                ("alacritty", Some(dir)) => format!("{program} --working-directory {dir} -e {run}"),
                ("wezterm", Some(dir)) => format!("{program} start --cwd {dir} -- {run}"),
                ("wezterm", None) => format!("{program} start -- {run}"),
                ("foot", None) | ("kitty", None) => format!("{program} {run}"),
                // xterm and anything unknown: -e is the convention; cd inside the shell.
                (_, Some(dir)) => format!(
                    "{program} -e sh -c {}",
                    Shell::Posix.quote(&format!("cd {dir} && {command}"))?
                ),
                (_, None) => format!("{program} -e {run}"),
            })
        }
    }
}

fn applescript_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        PathBuf::from("/home/u/my proj")
    }

    #[test]
    fn linux_terminals_each_get_their_own_spelling() {
        let cases = [
            (
                "ghostty",
                "ghostty --working-directory='/home/u/my proj' -e sh -c gitui",
            ),
            (
                "foot",
                "foot --working-directory='/home/u/my proj' sh -c gitui",
            ),
            ("kitty", "kitty --directory '/home/u/my proj' sh -c gitui"),
            (
                "alacritty",
                "alacritty --working-directory '/home/u/my proj' -e sh -c gitui",
            ),
            (
                "wezterm",
                "wezterm start --cwd '/home/u/my proj' -- sh -c gitui",
            ),
            (
                "xterm",
                "xterm -e sh -c 'cd '\\''/home/u/my proj'\\'' && gitui'",
            ),
        ];
        for (program, want) in cases {
            assert_eq!(
                wrap_for(Host::Linux, program, "gitui", Some(&dir())).unwrap(),
                want
            );
        }
        assert_eq!(
            wrap_for(Host::Linux, "/usr/bin/ghostty", "gitui", None).unwrap(),
            "/usr/bin/ghostty -e sh -c gitui"
        );
    }

    #[test]
    fn linux_without_any_terminal_is_an_error_that_says_what_to_do() {
        let error = wrap_for(Host::Linux, "", "gitui", None).unwrap_err();
        assert!(error.to_string().contains("$TERMINAL"));
    }

    #[test]
    fn macos_goes_through_applescript_with_quotes_escaped() {
        let line = wrap_for(Host::Macos, "Terminal", "echo \"hi\"", Some(&dir())).unwrap();
        assert!(line.starts_with("osascript -e "), "{line}");
        assert!(
            line.contains(r#"tell application "Terminal" to do script"#),
            "{line}"
        );
        assert!(
            line.contains(r#"cd '\''/home/u/my proj'\'' && echo \"hi\""#),
            "{line}"
        );
    }

    #[test]
    fn windows_prefers_windows_terminal_and_falls_back_to_start() {
        // Quoting here follows the build host's shell; the shape is what matters.
        let wt = wrap_for(Host::Windows, "wt", "gitui", Some(Path::new(r"C:\proj"))).unwrap();
        assert!(wt.starts_with("wt -d "), "{wt}");
        assert!(wt.ends_with(" cmd /k gitui"), "{wt}");

        let cmd = wrap_for(Host::Windows, "cmd", "gitui", Some(Path::new(r"C:\proj"))).unwrap();
        assert!(cmd.starts_with("start \"\" cmd /k \"cd /d "), "{cmd}");
        assert!(cmd.ends_with(" && gitui\""), "{cmd}");
    }
}
