//! Turning "this tool is missing" into the exact command that installs it.
//!
//! Detects the package managers on `PATH`, ranks them for the OS — a distro package
//! where one exists (fast, no compile; `paru`/`yay` before `pacman` because they handle
//! `sudo` themselves), then `cargo`, then `brew`, then a GitHub release via `eget`, then
//! `pipx` — and produces one command line per tool. Nothing here runs anything until
//! [`run`] is called with a list the user has already seen in full.

use std::fmt;

use anyhow::{Context, Result, bail};

use crate::catalog::Tool;
use crate::platform::Host;
use crate::runner::find_executable;
use crate::shell::Shell;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Manager {
    Paru,
    Yay,
    Pacman,
    Apt,
    Dnf,
    Brew,
    Cargo,
    Eget,
    Pipx,
    Winget,
    Scoop,
}

impl Manager {
    pub fn executable(self) -> &'static str {
        match self {
            Manager::Paru => "paru",
            Manager::Yay => "yay",
            Manager::Pacman => "pacman",
            Manager::Apt => "apt-get",
            Manager::Dnf => "dnf",
            Manager::Brew => "brew",
            Manager::Cargo => "cargo",
            Manager::Eget => "eget",
            Manager::Pipx => "pipx",
            Manager::Winget => "winget",
            Manager::Scoop => "scoop",
        }
    }

    /// Needs `sudo` in front (asked for interactively, in the foreground).
    fn needs_sudo(self) -> bool {
        matches!(self, Manager::Pacman | Manager::Apt | Manager::Dnf)
    }

    /// Preference order per OS. Only managers actually on `PATH` are considered.
    pub fn ranked_for(host: Host) -> &'static [Manager] {
        match host {
            Host::Linux | Host::OtherUnix => &[
                Manager::Paru,
                Manager::Yay,
                Manager::Pacman,
                Manager::Apt,
                Manager::Dnf,
                Manager::Brew,
                Manager::Cargo,
                Manager::Eget,
                Manager::Pipx,
            ],
            Host::Macos => &[Manager::Brew, Manager::Cargo, Manager::Eget, Manager::Pipx],
            Host::Windows => &[
                Manager::Winget,
                Manager::Scoop,
                Manager::Cargo,
                Manager::Eget,
                Manager::Pipx,
            ],
        }
    }

    /// The managers present on this machine.
    pub fn detect() -> Vec<Manager> {
        [
            Manager::Paru,
            Manager::Yay,
            Manager::Pacman,
            Manager::Apt,
            Manager::Dnf,
            Manager::Brew,
            Manager::Cargo,
            Manager::Eget,
            Manager::Pipx,
            Manager::Winget,
            Manager::Scoop,
        ]
        .into_iter()
        .filter(|manager| find_executable(manager.executable()).is_some())
        .collect()
    }
}

impl fmt::Display for Manager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.executable())
    }
}

/// One tool, one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub tool: String,
    pub manager: Manager,
    pub command: String,
}

/// What to run for `tool`, or `None` when no available manager knows it — the caller
/// then shows the tool's `note`/`homepage` instead.
pub fn plan(tool: &Tool, available: &[Manager], host: Host) -> Option<Step> {
    for manager in Manager::ranked_for(host) {
        if !available.contains(manager) {
            continue;
        }
        let install = &tool.install;
        let package = match manager {
            Manager::Paru | Manager::Yay | Manager::Pacman => install.pacman.as_deref(),
            Manager::Apt => install.apt.as_deref(),
            Manager::Dnf => install.dnf.as_deref(),
            Manager::Brew => install.brew.as_deref(),
            Manager::Cargo => install.cargo.as_deref(),
            Manager::Eget => install.github.as_deref(),
            Manager::Pipx => install.pipx.as_deref(),
            Manager::Winget => install.winget.as_deref(),
            Manager::Scoop => install.scoop.as_deref(),
        };
        let Some(package) = package else { continue };
        let sudo = if manager.needs_sudo() { "sudo " } else { "" };
        let command = match manager {
            Manager::Paru | Manager::Yay => format!("{manager} -S --needed {package}"),
            Manager::Pacman => format!("{sudo}pacman -S --needed {package}"),
            Manager::Apt => format!("{sudo}apt-get install -y {package}"),
            Manager::Dnf => format!("{sudo}dnf install -y {package}"),
            Manager::Brew => format!("brew install {package}"),
            Manager::Cargo => format!("cargo install --locked {package}"),
            Manager::Eget => format!("eget {package} --to ~/.local/bin"),
            Manager::Pipx => format!("pipx install {package}"),
            Manager::Winget => format!("winget install --id {package}"),
            Manager::Scoop => format!("scoop install {package}"),
        };
        return Some(Step {
            tool: tool.name.clone(),
            manager: *manager,
            command,
        });
    }
    None
}

/// Run the steps in order, in the foreground so package-manager output and any `sudo`
/// prompt reach the user. Stops at the first failure and prints what is left to run by
/// hand — the config has already been written, so nothing is lost.
pub fn run(steps: &[Step]) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        eprintln!("\n==> {} ({})", step.command, step.tool);
        let status = Shell::current()
            .command(&step.command)
            .status()
            .with_context(|| format!("could not start `{}`", step.command))?;
        if !status.success() {
            let remaining: Vec<&str> = steps[index + 1..]
                .iter()
                .map(|s| s.command.as_str())
                .collect();
            if remaining.is_empty() {
                bail!("`{}` exited with {status}", step.command);
            }
            bail!(
                "`{}` exited with {status}; not run:\n  {}",
                step.command,
                remaining.join("\n  ")
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Install;

    fn tool(name: &str, install: Install) -> Tool {
        Tool {
            name: name.to_string(),
            summary: String::new(),
            homepage: None,
            binary: None,
            platforms: Vec::new(),
            install,
        }
    }

    #[test]
    fn arch_prefers_paru_over_pacman_over_cargo() {
        let xan = tool(
            "xan",
            Install {
                cargo: Some("xan".into()),
                pacman: Some("xan".into()),
                ..Install::default()
            },
        );
        let step = plan(
            &xan,
            &[Manager::Cargo, Manager::Pacman, Manager::Paru],
            Host::Linux,
        )
        .unwrap();
        assert_eq!(step.command, "paru -S --needed xan");

        let step = plan(&xan, &[Manager::Cargo, Manager::Pacman], Host::Linux).unwrap();
        assert_eq!(step.command, "sudo pacman -S --needed xan");

        let step = plan(&xan, &[Manager::Cargo], Host::Linux).unwrap();
        assert_eq!(step.command, "cargo install --locked xan");
    }

    #[test]
    fn a_tool_only_on_github_needs_eget_and_falls_through_otherwise() {
        let lazyenv = tool(
            "lazyenv",
            Install {
                github: Some("owner/lazyenv".into()),
                ..Install::default()
            },
        );
        assert_eq!(
            plan(&lazyenv, &[Manager::Paru, Manager::Cargo], Host::Linux),
            None
        );
        let step = plan(&lazyenv, &[Manager::Eget], Host::Linux).unwrap();
        assert_eq!(step.command, "eget owner/lazyenv --to ~/.local/bin");
    }

    #[test]
    fn macos_and_windows_have_their_own_order() {
        let yazi = tool(
            "yazi",
            Install {
                brew: Some("yazi".into()),
                cargo: Some("yazi-fm yazi-cli".into()),
                winget: Some("sxyazi.yazi".into()),
                ..Install::default()
            },
        );
        let mac = plan(&yazi, &[Manager::Cargo, Manager::Brew], Host::Macos).unwrap();
        assert_eq!(mac.command, "brew install yazi");
        let win = plan(&yazi, &[Manager::Cargo, Manager::Winget], Host::Windows).unwrap();
        assert_eq!(win.command, "winget install --id sxyazi.yazi");
        let win_no_winget = plan(&yazi, &[Manager::Cargo], Host::Windows).unwrap();
        assert_eq!(
            win_no_winget.command,
            "cargo install --locked yazi-fm yazi-cli"
        );
    }
}
