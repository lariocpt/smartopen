//! `smartopen yazi …` and `smartopen broot …`: make the file managers open files through
//! this menu — or through explicit per-type viewers from the same spec.
//!
//! Both navigators are driven from one [`Spec`]: the built-in default or an external
//! `--spec` file, transformed by the engine (`--rules` keeps explicit openers; the default
//! delegates every file to this binary, which shows its own menu). yazi gets its
//! `[opener]`/`[open]` tables spliced into `yazi.toml`; broot gets an Enter verb in
//! `smartopen.hjson` imported from `conf.hjson`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::engine::{self, Engine};
use crate::platform::Host;
use crate::spec::{self, RawSpec, Spec};
use crate::{broot, diff, paths, render, tomlio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigator {
    Yazi,
    Broot,
}

impl Navigator {
    fn name(self) -> &'static str {
        match self {
            Navigator::Yazi => "yazi",
            Navigator::Broot => "broot",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Write the configuration; `force` replaces hand-edited sections.
    Apply {
        force: bool,
        no_backup: bool,
    },
    Diff,
    Check,
    Print,
    PrintSpec,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The binary the navigator delegates to. Defaults to whichever one is running, so an
    /// `opn`-only install produces a config that calls `opn`.
    pub bin: Option<String>,
    /// `yazi.toml` or broot's config directory, instead of the platform default.
    pub target: Option<PathBuf>,
    /// Explicit per-type openers instead of delegating everything to the menu.
    pub rules: bool,
    /// An external spec file instead of the built-in default.
    pub spec: Option<PathBuf>,
}

/// The exit code: 0, or 1 from `check` when the target would change.
pub fn run(navigator: Navigator, action: Action, options: &Options) -> Result<i32> {
    if action == Action::PrintSpec {
        // Always the built-in default, in editable form: a starting template.
        print!("{}", spec::spec_to_file_string(&Spec::builtin())?);
        return Ok(0);
    }
    if options.spec.is_some() && !options.rules {
        bail!(
            "--spec only applies with --rules: without it every file is delegated to the menu and the spec is not read"
        );
    }
    let base = match &options.spec {
        Some(path) => {
            let text = fs::read_to_string(path)
                .with_context(|| format!("reading spec file {}", path.display()))?;
            RawSpec::parse(&text)
                .with_context(|| format!("parsing spec file {}", path.display()))?
        }
        None => Spec::builtin(),
    };
    base.validate().context("spec failed validation")?;

    let bin = options.bin.clone().unwrap_or_else(current_bin_name);
    let engine = if options.rules {
        Engine::Rules
    } else {
        Engine::Smartopen
    };
    let effective = engine::effective(&base, engine, &bin);

    match navigator {
        Navigator::Yazi => yazi(action, options, &effective),
        Navigator::Broot => broot_cmd(action, options, &effective),
    }
}

fn yazi(action: Action, options: &Options, effective: &Spec) -> Result<i32> {
    let config_path = match &options.target {
        Some(path) => path.clone(),
        None => paths::yazi_config_path().context("could not determine yazi's config directory")?,
    };

    match action {
        Action::Print => {
            print!("{}", render::fragment(effective));
        }
        Action::Diff => {
            let existing = tomlio::read_optional(&config_path)?;
            let new_text = tomlio::render_to_string(existing.as_deref(), effective)?;
            let old = existing.unwrap_or_default();
            if old == new_text {
                println!("# {} is already in sync", config_path.display());
            } else {
                print!("{}", diff::unified(&old, &new_text));
            }
        }
        Action::Check => {
            let existing = tomlio::read_optional(&config_path)?;
            let new_text = tomlio::render_to_string(existing.as_deref(), effective)?;
            if existing.as_deref() == Some(new_text.as_str()) {
                println!("in sync: {}", config_path.display());
            } else {
                println!(
                    "drift: {} would change (run `smartopen yazi apply`)",
                    config_path.display()
                );
                return Ok(1);
            }
        }
        Action::Apply { force, no_backup } => {
            match tomlio::apply(&config_path, effective, no_backup, force)? {
                tomlio::Outcome::Created => println!("created {}", config_path.display()),
                tomlio::Outcome::Updated => println!("updated {}", config_path.display()),
                tomlio::Outcome::InSync => {
                    println!("already in sync: {}", config_path.display())
                }
            }
        }
        Action::PrintSpec => unreachable!("handled before dispatch"),
    }
    Ok(0)
}

fn broot_cmd(action: Action, options: &Options, effective: &Spec) -> Result<i32> {
    // broot runs a verb's `external` without a shell, and the Enter verb is an `sh -c`
    // dispatcher with no cmd equivalent. Refusing beats writing a verb that binds Enter
    // to a program Windows does not have.
    if Host::current() == Host::Windows {
        bail!(
            "broot integration is not available on Windows: broot runs verbs without a shell, and the smartopen verb needs `sh`"
        );
    }
    let dir = match &options.target {
        Some(path) => path.clone(),
        None => {
            paths::broot_config_dir().context("could not determine broot's config directory")?
        }
    };
    let openers_path = dir.join(broot::OPENERS_FILE);
    let conf_path = dir.join("conf.hjson");

    match action {
        Action::Print => {
            print!("{}", broot::openers_hjson(effective));
        }
        Action::Diff => {
            let (new_conf, openers_change) = broot::render(&dir, effective)?;
            let old_conf = tomlio::read_optional(&conf_path)?.unwrap_or_default();
            let mut changed = false;
            if let Some(new_openers) = openers_change {
                let old = tomlio::read_optional(&openers_path)?.unwrap_or_default();
                print!(
                    "{}",
                    diff::unified(&old, &new_openers)
                        .replace("a/yazi.toml", &format!("a/{}", broot::OPENERS_FILE))
                        .replace("b/yazi.toml", &format!("b/{}", broot::OPENERS_FILE))
                );
                changed = true;
            }
            if old_conf != new_conf {
                print!(
                    "{}",
                    diff::unified(&old_conf, &new_conf)
                        .replace("a/yazi.toml", "a/conf.hjson")
                        .replace("b/yazi.toml", "b/conf.hjson")
                );
                changed = true;
            }
            if !changed {
                println!("# {} is already in sync", dir.display());
            }
        }
        Action::Check => {
            let (new_conf, openers_change) = broot::render(&dir, effective)?;
            let old_conf = tomlio::read_optional(&conf_path)?.unwrap_or_default();
            if openers_change.is_none() && old_conf == new_conf {
                println!("in sync: {}", dir.display());
            } else {
                println!(
                    "drift: {} would change (run `smartopen broot apply`)",
                    dir.display()
                );
                return Ok(1);
            }
        }
        Action::Apply { no_backup, .. } => {
            let (outcome, import_added) = broot::apply(&dir, effective, false, no_backup)?;
            match outcome {
                tomlio::Outcome::Created => println!("created {}", openers_path.display()),
                tomlio::Outcome::Updated => println!("updated {}", openers_path.display()),
                tomlio::Outcome::InSync => {
                    println!("already in sync: {}", openers_path.display())
                }
            }
            if import_added {
                println!(
                    "added {} import to {}",
                    broot::OPENERS_FILE,
                    conf_path.display()
                );
            }
        }
        Action::PrintSpec => unreachable!("handled before dispatch"),
    }
    let _ = Navigator::Broot.name();
    Ok(0)
}

/// The name this program was invoked as — `smartopen` or `opn` — for the delegate.
pub fn current_bin_name() -> String {
    std::env::args()
        .next()
        .and_then(|arg0| {
            Path::new(&arg0)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "smartopen".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("smartopen-nav-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn yazi_check_is_1_before_apply_and_0_after() {
        let dir = temp_dir("yazi");
        let options = Options {
            bin: Some("opn".to_string()),
            target: Some(dir.join("yazi.toml")),
            ..Options::default()
        };

        assert_eq!(run(Navigator::Yazi, Action::Check, &options).unwrap(), 1);
        assert_eq!(
            run(
                Navigator::Yazi,
                Action::Apply {
                    force: true,
                    no_backup: true
                },
                &options
            )
            .unwrap(),
            0
        );
        assert_eq!(run(Navigator::Yazi, Action::Check, &options).unwrap(), 0);

        let written = fs::read_to_string(dir.join("yazi.toml")).unwrap();
        assert!(
            written.contains("run = 'opn %s'"),
            "delegates to the named bin:\n{written}"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn broot_check_is_1_before_apply_and_0_after() {
        let dir = temp_dir("broot");
        let options = Options {
            bin: Some("smartopen".to_string()),
            target: Some(dir.clone()),
            ..Options::default()
        };

        if cfg!(windows) {
            // Refused there: broot runs verbs without a shell and the Enter verb needs `sh`.
            let error = run(Navigator::Broot, Action::Check, &options).unwrap_err();
            assert!(error.to_string().contains("Windows"), "{error}");
            let _ = fs::remove_dir_all(dir);
            return;
        }

        assert_eq!(run(Navigator::Broot, Action::Check, &options).unwrap(), 1);
        run(
            Navigator::Broot,
            Action::Apply {
                force: true,
                no_backup: true,
            },
            &options,
        )
        .unwrap();
        assert_eq!(run(Navigator::Broot, Action::Check, &options).unwrap(), 0);
        assert!(dir.join(broot::OPENERS_FILE).is_file());
        assert!(dir.join("conf.hjson").is_file());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rules_engine_writes_explicit_openers_instead_of_the_delegate() {
        let dir = temp_dir("rules");
        let options = Options {
            bin: Some("smartopen".to_string()),
            target: Some(dir.join("yazi.toml")),
            rules: true,
            ..Options::default()
        };
        run(
            Navigator::Yazi,
            Action::Apply {
                force: true,
                no_backup: true,
            },
            &options,
        )
        .unwrap();
        let written = fs::read_to_string(dir.join("yazi.toml")).unwrap();
        assert!(written.contains("mdfried"), "{written}");
        assert!(!written.contains("smartopen \"$@\""), "{written}");
        let _ = fs::remove_dir_all(dir);
    }
}
