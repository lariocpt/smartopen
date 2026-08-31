//! Where smartopen — and the tools it configures — keep their files, on each OS.
//!
//! `$XDG_CONFIG_HOME`, falling back to `~/.config`, on every Unix INCLUDING macOS: this
//! is a terminal program and `~/.config` is where its users look, not
//! `~/Library/Application Support`. `%APPDATA%` on Windows. yazi and broot make the same
//! choice, so their paths resolve through the same rule.
//!
//! Every resolver returns `Option`. When there is nowhere sensible to write, say so,
//! rather than falling back to the current directory and scattering files wherever the
//! user happened to launch from — on Windows `HOME` is normally unset.

use std::env;
use std::path::PathBuf;

pub const APP_DIR: &str = "smartopen";
/// The directory this tool used before it was published. Honoured when it exists and the
/// new one does not, so nobody has to move a config to upgrade.
pub const LEGACY_APP_DIR: &str = "opn";

/// `…/smartopen/config.toml`, or the legacy `…/opn/config.toml` if only that exists.
pub fn config_path() -> Option<PathBuf> {
    let base = config_base()?;
    let current = base.join(APP_DIR).join("config.toml");
    if !current.exists() {
        let legacy = base.join(LEGACY_APP_DIR).join("config.toml");
        if legacy.exists() {
            return Some(legacy);
        }
    }
    Some(current)
}

/// yazi's `yazi.toml`: `~/.config/yazi/yazi.toml` on Unix and macOS,
/// `%APPDATA%\yazi\config\yazi.toml` on Windows.
pub fn yazi_config_path() -> Option<PathBuf> {
    let base = config_base()?.join("yazi");
    #[cfg(windows)]
    let base = base.join("config");
    Some(base.join("yazi.toml"))
}

fn config_base() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        resolve_base(env::var_os("APPDATA").map(PathBuf::from), None, &[])
    }
    #[cfg(not(windows))]
    {
        resolve_base(
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            home_dir(),
            &[".config"],
        )
    }
}

#[cfg(not(windows))]
fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

/// The pure part of the path rules, so they can be tested without mutating the process
/// environment (which is `unsafe` in edition 2024 and racy under a parallel test runner).
///
/// A relative `explicit` is ignored: the XDG spec says a non-absolute `XDG_*_HOME` must
/// be treated as unset.
fn resolve_base(
    explicit: Option<PathBuf>,
    home: Option<PathBuf>,
    home_rel: &[&str],
) -> Option<PathBuf> {
    explicit
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home_rel.iter().fold(home, |acc, part| acc.join(part))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_absolute_base_wins() {
        let base = resolve_base(
            Some(PathBuf::from("/x/cfg")),
            Some(PathBuf::from("/home/u")),
            &[".config"],
        );
        assert_eq!(base, Some(PathBuf::from("/x/cfg")));
    }

    #[test]
    fn relative_explicit_base_is_treated_as_unset() {
        let base = resolve_base(
            Some(PathBuf::from("cfg")),
            Some(PathBuf::from("/home/u")),
            &[".config"],
        );
        assert_eq!(base, Some(PathBuf::from("/home/u/.config")));
    }

    #[test]
    fn nothing_to_resolve_from_is_none_not_dot() {
        assert_eq!(resolve_base(None, None, &[".config"]), None);
    }

    #[test]
    fn config_path_ends_in_the_app_dir_on_this_os() {
        let path = config_path().expect("a config dir exists on the test machine");
        let dir = path.parent().unwrap().file_name().unwrap();
        assert!(
            dir == APP_DIR || dir == LEGACY_APP_DIR,
            "{}",
            path.display()
        );
        assert_eq!(path.file_name().unwrap(), "config.toml");
        if cfg!(target_os = "macos") {
            assert!(
                !path
                    .to_string_lossy()
                    .contains("Library/Application Support"),
                "macOS must use ~/.config, got {}",
                path.display()
            );
        }
    }
}
