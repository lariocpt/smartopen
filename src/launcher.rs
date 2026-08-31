//! `[shortcut.when]`: offer a command only where it makes sense.
//!
//! A launcher that knows where you are. `cwd_has = ["Cargo.toml"]` makes "Cargo test"
//! appear inside Rust projects and nowhere else; `has = ["gitui"]` makes a command vanish
//! on a machine without the tool instead of failing; `env = ["SSH_CONNECTION"]` marks
//! remote-only shortcuts. Every condition has to hold. `--all` shows what was hidden and
//! which condition hid it, so "why isn't it there" is always answerable.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::matcher::wildcard_matches;
use crate::runner::find_executable;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// Any of these files or directories exists in the working directory or one of its
    /// ancestors, up to and including the directory that holds `.git`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cwd_has: Vec<String>,
    /// Globs the absolute working directory must match (any of them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cwd_matches: Vec<String>,
    /// `VAR` — set and non-empty — or `VAR=glob` (any of them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// Executables that must all be on `PATH`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub has: Vec<String>,
}

/// What the conditions are evaluated against. Built once per run from the real process;
/// built by hand in tests so nothing mutates the environment.
pub struct Context<'a> {
    pub cwd: PathBuf,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub on_path: &'a dyn Fn(&str) -> bool,
}

impl Context<'_> {
    pub fn from_process() -> Context<'static> {
        Context {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env: &|name| std::env::var(name).ok(),
            on_path: &|name| find_executable(name).is_some(),
        }
    }
}

impl When {
    pub fn is_empty(&self) -> bool {
        *self == When::default()
    }

    /// `Ok(())` when every condition holds; otherwise the first failing condition,
    /// worded for the `--all` display.
    pub fn check(&self, context: &Context<'_>) -> Result<(), String> {
        if !self.cwd_has.is_empty()
            && !self
                .cwd_has
                .iter()
                .any(|name| found_upward(&context.cwd, name))
        {
            return Err(format!("cwd_has {:?}", self.cwd_has));
        }

        if !self.cwd_matches.is_empty() {
            let cwd = context.cwd.display().to_string();
            if !self
                .cwd_matches
                .iter()
                .any(|pattern| wildcard_matches(pattern, &cwd))
            {
                return Err(format!("cwd_matches {:?}", self.cwd_matches));
            }
        }

        if !self.env.is_empty() && !self.env.iter().any(|spec| env_matches(spec, context.env)) {
            return Err(format!("env {:?}", self.env));
        }

        if let Some(missing) = self.has.iter().find(|name| !(context.on_path)(name)) {
            return Err(format!("has {missing:?} (not on PATH)"));
        }

        Ok(())
    }
}

/// Does `name` exist in `start` or an ancestor? The walk stops after the directory that
/// holds `.git`, so a `Cargo.toml` in a parent project does not claim its subprojects.
fn found_upward(start: &Path, name: &str) -> bool {
    let mut dir = Some(start);
    while let Some(current) = dir {
        if current.join(name).exists() {
            return true;
        }
        if current.join(".git").exists() {
            return false;
        }
        dir = current.parent();
    }
    false
}

fn env_matches(spec: &str, env: &dyn Fn(&str) -> Option<String>) -> bool {
    match spec.split_once('=') {
        Some((name, pattern)) => env(name).is_some_and(|value| wildcard_matches(pattern, &value)),
        None => env(spec).is_some_and(|value| !value.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    type EnvFn<'a> = Box<dyn Fn(&str) -> Option<String> + 'a>;
    type PathFn<'a> = Box<dyn Fn(&str) -> bool + 'a>;

    fn context<'a>(
        cwd: PathBuf,
        env: &'a HashMap<&str, &str>,
        path: &'a [&str],
    ) -> (PathBuf, EnvFn<'a>, PathFn<'a>) {
        (
            cwd,
            Box::new(move |name| env.get(name).map(|v| v.to_string())),
            Box::new(move |name| path.contains(&name)),
        )
    }

    fn temp_tree(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("smartopen-when-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("repo").join("src").join("deep")).unwrap();
        fs::create_dir_all(root.join("repo").join(".git")).unwrap();
        fs::write(root.join("repo").join("Cargo.toml"), "").unwrap();
        fs::write(root.join("package.json"), "").unwrap();
        root
    }

    #[test]
    fn cwd_has_looks_upward_but_not_past_git() {
        let root = temp_tree("cwd");
        let env = HashMap::new();
        let (cwd, env_fn, path_fn) = context(root.join("repo/src/deep"), &env, &[]);
        let ctx = Context {
            cwd,
            env: &*env_fn,
            on_path: &*path_fn,
        };

        let rust = When {
            cwd_has: vec!["Cargo.toml".into()],
            ..When::default()
        };
        assert_eq!(rust.check(&ctx), Ok(()));

        // package.json is above the repo's .git — invisible from inside it.
        let node = When {
            cwd_has: vec!["package.json".into()],
            ..When::default()
        };
        assert_eq!(
            node.check(&ctx),
            Err("cwd_has [\"package.json\"]".to_string())
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn env_accepts_set_or_glob_and_rejects_empty() {
        let env: HashMap<&str, &str> = [("TMUX", "/tmp/tmux-1000"), ("EMPTY", "")]
            .into_iter()
            .collect();
        let (cwd, env_fn, path_fn) = context(PathBuf::from("/"), &env, &[]);
        let ctx = Context {
            cwd,
            env: &*env_fn,
            on_path: &*path_fn,
        };

        let check = |spec: &str| {
            When {
                env: vec![spec.to_string()],
                ..When::default()
            }
            .check(&ctx)
            .is_ok()
        };
        assert!(check("TMUX"));
        assert!(check("TMUX=/tmp/*"));
        assert!(!check("TMUX=/var/*"));
        assert!(!check("EMPTY"), "set but empty is not set");
        assert!(!check("SSH_CONNECTION"));
    }

    #[test]
    fn has_and_cwd_matches_name_what_failed() {
        let env = HashMap::new();
        let (cwd, env_fn, path_fn) = context(PathBuf::from("/home/u/work/app"), &env, &["git"]);
        let ctx = Context {
            cwd,
            env: &*env_fn,
            on_path: &*path_fn,
        };

        let tools = When {
            has: vec!["git".into(), "gitui".into()],
            ..When::default()
        };
        assert_eq!(
            tools.check(&ctx),
            Err("has \"gitui\" (not on PATH)".to_string())
        );

        let place = When {
            cwd_matches: vec!["/home/u/work/*".into()],
            ..When::default()
        };
        assert_eq!(place.check(&ctx), Ok(()));
        let elsewhere = When {
            cwd_matches: vec!["/srv/*".into()],
            ..When::default()
        };
        assert!(elsewhere.check(&ctx).is_err());
    }

    #[test]
    fn empty_when_always_holds() {
        let env = HashMap::new();
        let (cwd, env_fn, path_fn) = context(PathBuf::from("/"), &env, &[]);
        let ctx = Context {
            cwd,
            env: &*env_fn,
            on_path: &*path_fn,
        };
        assert!(When::default().is_empty());
        assert_eq!(When::default().check(&ctx), Ok(()));
    }
}
