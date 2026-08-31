//! Which operating system a command is for.
//!
//! One config file serves three OSes: a command can carry `platform = "macos"` and it is
//! simply not offered anywhere else. The vocabulary mirrors yazi's `for` field
//! (`unix` | `linux` | `macos` | `windows`) so the two configurations read the same way.

use serde::{Deserialize, Serialize};

/// A `platform = "…"` value on a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Any Unix-like OS: Linux, macOS, the BSDs.
    Unix,
    Linux,
    Macos,
    Windows,
}

impl Platform {
    /// Does a command marked with this platform apply on the OS this binary runs on?
    pub fn applies_here(self) -> bool {
        self.applies_on(Host::current())
    }

    pub fn applies_on(self, host: Host) -> bool {
        match self {
            Platform::Unix => host != Host::Windows,
            Platform::Linux => host == Host::Linux,
            Platform::Macos => host == Host::Macos,
            Platform::Windows => host == Host::Windows,
        }
    }
}

/// The OS this binary is running on, reduced to what the config vocabulary distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Linux,
    Macos,
    Windows,
    /// A Unix that is neither Linux nor macOS (the BSDs); matches `unix` only.
    OtherUnix,
}

impl Host {
    pub fn current() -> Host {
        if cfg!(target_os = "linux") {
            Host::Linux
        } else if cfg!(target_os = "macos") {
            Host::Macos
        } else if cfg!(windows) {
            Host::Windows
        } else {
            Host::OtherUnix
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Host::Linux => "linux",
            Host::Macos => "macos",
            Host::Windows => "windows",
            Host::OtherUnix => "unix",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_covers_every_non_windows_host() {
        for host in [Host::Linux, Host::Macos, Host::OtherUnix] {
            assert!(Platform::Unix.applies_on(host), "{host:?}");
        }
        assert!(!Platform::Unix.applies_on(Host::Windows));
    }

    #[test]
    fn specific_platforms_match_only_themselves() {
        assert!(Platform::Linux.applies_on(Host::Linux));
        assert!(!Platform::Linux.applies_on(Host::Macos));
        assert!(Platform::Macos.applies_on(Host::Macos));
        assert!(!Platform::Macos.applies_on(Host::OtherUnix));
        assert!(Platform::Windows.applies_on(Host::Windows));
        assert!(!Platform::Windows.applies_on(Host::Linux));
    }

    #[test]
    fn platform_parses_from_the_yazi_vocabulary() {
        #[derive(Deserialize)]
        struct Row {
            platform: Platform,
        }
        for (text, want) in [
            ("unix", Platform::Unix),
            ("linux", Platform::Linux),
            ("macos", Platform::Macos),
            ("windows", Platform::Windows),
        ] {
            let row: Row = toml::from_str(&format!("platform = \"{text}\"")).unwrap();
            assert_eq!(row.platform, want);
        }
        assert!(toml::from_str::<Row>("platform = \"darwin\"").is_err());
    }
}
