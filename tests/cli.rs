//! L2: the command-line surface, on every OS. Each test runs the real binary in a
//! sandboxed home, so nothing here can touch a developer's `~/.config`.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// A throwaway home: config, state and (on Windows) AppData all point into it.
struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        Sandbox {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn config_dir(&self) -> PathBuf {
        self.path().join("config")
    }

    /// Where `config path` should land on this OS.
    fn expected_config_path(&self) -> PathBuf {
        self.config_dir().join("smartopen").join("config.toml")
    }

    fn write_config(&self, text: &str) -> PathBuf {
        let path = self.expected_config_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn cmd(&self, bin: &str) -> Command {
        let mut cmd = Command::cargo_bin(bin).expect("binary built");
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.path())
            .env("USERPROFILE", self.path())
            .env("XDG_CONFIG_HOME", self.config_dir())
            .env("XDG_STATE_HOME", self.path().join("state"))
            .env("APPDATA", self.config_dir())
            .env("LOCALAPPDATA", self.path().join("state"))
            // broot does not follow XDG on macOS; this override is honoured everywhere.
            .env("BROOT_CONFIG_DIR", self.config_dir().join("broot"))
            .env("TERM", "dumb")
            .current_dir(self.path());
        cmd
    }

    fn smartopen(&self) -> Command {
        self.cmd("smartopen")
    }
}

fn csv_config(run: &str) -> String {
    format!(
        "[[extension]]\nextensions = [\"csv\"]\n\n[[extension.command]]\nlabel = \"Only\"\nrun = \"{run}\"\n"
    )
}

#[test]
fn both_binaries_report_the_same_version() {
    let sandbox = Sandbox::new();
    let want = format!("smartopen {}\n", env!("CARGO_PKG_VERSION"));
    for bin in ["smartopen", "opn"] {
        sandbox
            .cmd(bin)
            .arg("--version")
            .assert()
            .success()
            .stdout(want.clone());
    }
}

#[test]
fn config_path_follows_xdg_everywhere_and_appdata_on_windows() {
    let sandbox = Sandbox::new();
    let out = sandbox
        .smartopen()
        .args(["config", "path"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let printed = String::from_utf8(out).unwrap();
    let first = printed.lines().next().unwrap();
    assert_eq!(Path::new(first), sandbox.expected_config_path());
    if cfg!(target_os = "macos") {
        assert!(!first.contains("Library/Application Support"), "{first}");
    }
}

#[test]
fn sample_then_list_then_doctor_all_succeed() {
    let sandbox = Sandbox::new();
    let sample = sandbox
        .smartopen()
        .args(["config", "sample"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let path = sandbox.write_config(std::str::from_utf8(&sample).unwrap());

    sandbox
        .smartopen()
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Shortcuts:"));

    // Doctor reports and exits 0 by default; it only fails under --strict.
    sandbox
        .smartopen()
        .args(["config", "doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Doctor:"));

    // The legacy spelling still works.
    sandbox
        .smartopen()
        .args(["--config-path", path.to_str().unwrap(), "--doctor"])
        .assert()
        .success();
}

#[test]
fn dry_run_quotes_a_spaced_path_for_the_platform_shell() {
    let sandbox = Sandbox::new();
    sandbox.write_config(&csv_config("echo {path}"));
    let dir = sandbox.path().join("space dir");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("my file.csv");
    fs::write(&file, "a,b\n").unwrap();

    let out = sandbox
        .smartopen()
        .arg("--dry-run")
        .arg(&file)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let printed = String::from_utf8(out).unwrap();

    if cfg!(windows) {
        assert!(printed.contains("command: echo \""), "{printed}");
        assert!(printed.contains("my file.csv\""), "{printed}");
        assert!(!printed.contains(r"\\?\"), "no UNC prefix: {printed}");
    } else {
        assert!(printed.contains("command: echo '"), "{printed}");
        assert!(printed.contains("/space dir/my file.csv'"), "{printed}");
    }
}

#[test]
fn the_launched_commands_exit_code_is_ours() {
    let sandbox = Sandbox::new();
    let run = if cfg!(windows) {
        "cmd /c exit 7"
    } else {
        "sh -c 'exit 7'"
    };
    sandbox.write_config(&format!(
        "[[shortcut]]\nlabel = \"Fail\"\nrun = \"{}\"\n",
        run.replace('"', "\\\"")
    ));
    sandbox
        .smartopen()
        .args(["--command", "Fail", "--no-history"])
        .assert()
        .code(7);
}

#[test]
fn no_config_without_a_terminal_names_the_wizard() {
    let sandbox = Sandbox::new();
    sandbox
        .smartopen()
        .assert()
        .failure()
        .stderr(predicate::str::contains("smartopen wizard"));
}

#[test]
fn a_misspelt_key_is_rejected_with_the_field_named() {
    let sandbox = Sandbox::new();
    sandbox.write_config("[[shortcut]]\nlabel = \"x\"\nrun = \"x\"\ndettach = true\n");
    sandbox
        .smartopen()
        .args(["config", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown field `dettach`"));
}

#[test]
fn wizard_yes_dry_run_writes_nothing_and_runs_nothing() {
    let sandbox = Sandbox::new();
    sandbox
        .smartopen()
        .args(["wizard", "--yes", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "--dry-run: nothing written, nothing run.",
        ));
    assert!(
        !sandbox.config_dir().exists(),
        "dry-run must not create anything"
    );
}

#[test]
fn wizard_without_a_terminal_or_yes_refuses() {
    let sandbox = Sandbox::new();
    sandbox
        .smartopen()
        .arg("wizard")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--yes"));
}

#[test]
fn wizard_yes_no_install_writes_a_config_doctor_accepts() {
    let sandbox = Sandbox::new();
    sandbox
        .smartopen()
        .args(["wizard", "--yes", "--no-install"])
        .assert()
        .success();
    assert!(sandbox.expected_config_path().is_file());
    sandbox
        .smartopen()
        .args(["config", "doctor"])
        .assert()
        .success();
    // The navigators were configured into the sandbox, nowhere else. yazi keeps its
    // file under yazi/config/ on Windows; broot's dir is pinned by BROOT_CONFIG_DIR.
    let yazi_toml = if cfg!(windows) {
        sandbox
            .config_dir()
            .join("yazi")
            .join("config")
            .join("yazi.toml")
    } else {
        sandbox.config_dir().join("yazi").join("yazi.toml")
    };
    assert!(yazi_toml.is_file(), "{}", yazi_toml.display());
    // broot integration is not offered on Windows (the Enter verb needs `sh`).
    let broot_verbs = sandbox.config_dir().join("broot").join("smartopen.hjson");
    assert_eq!(
        broot_verbs.is_file(),
        !cfg!(windows),
        "{}",
        broot_verbs.display()
    );
}

#[test]
fn completions_man_and_shell_name_the_invoked_binary() {
    let sandbox = Sandbox::new();
    sandbox
        .cmd("opn")
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_opn"));
    sandbox
        .smartopen()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("#compdef smartopen"));
    sandbox
        .smartopen()
        .arg("man")
        .assert()
        .success()
        .stdout(predicate::str::contains(".TH smartopen 1"));
    for shell in ["zsh", "bash", "fish"] {
        sandbox
            .cmd("opn")
            .args(["shell", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("opn --print"));
    }
}

#[test]
fn shortcuts_import_navi_prints_parseable_toml() {
    let sandbox = Sandbox::new();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/git.cheat");
    let out = sandbox
        .smartopen()
        .args(["shortcuts", "import", "navi"])
        .arg(&fixture)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let value: toml::Value = toml::from_str(&text).expect("valid TOML");
    let shortcuts = value["shortcut"].as_array().unwrap();
    assert_eq!(shortcuts.len(), 2);
    assert_eq!(
        shortcuts[0]["run"].as_str(),
        Some("git checkout {{branch}}")
    );
    assert!(shortcuts[0]["param"]["branch"]["choices"].is_str());
}

#[test]
fn yazi_print_is_toml_and_broot_print_is_json() {
    let sandbox = Sandbox::new();
    let yazi = sandbox
        .smartopen()
        .args(["yazi", "print"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doc: toml::Value = toml::from_str(std::str::from_utf8(&yazi).unwrap()).unwrap();
    assert!(doc.get("opener").is_some() && doc.get("open").is_some());

    if cfg!(windows) {
        // Refused there, with the reason: broot runs verbs without a shell.
        sandbox
            .cmd("opn")
            .args(["broot", "print"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Windows"));
        return;
    }
    let broot = sandbox
        .cmd("opn")
        .args(["broot", "print"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(broot).unwrap();
    let json: serde_json::Value = serde_json::from_str(
        &text
            .lines()
            .filter(|l| !l.starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    assert_eq!(json["verbs"][0]["invocation"], "smartopen");
    // The delegate is the binary that produced it.
    assert!(text.contains("opn \\\"$@\\\""), "{text}");
}

#[test]
fn yazi_and_broot_check_apply_check_round_trip_in_a_temp_target() {
    let sandbox = Sandbox::new();
    let yazi_toml = sandbox.path().join("yazi.toml");
    sandbox
        .smartopen()
        .args(["yazi", "check", "--target"])
        .arg(&yazi_toml)
        .assert()
        .code(1);
    sandbox
        .smartopen()
        .args(["yazi", "apply", "--target"])
        .arg(&yazi_toml)
        .assert()
        .success();
    sandbox
        .smartopen()
        .args(["yazi", "check", "--target"])
        .arg(&yazi_toml)
        .assert()
        .success();

    let broot_dir = sandbox.path().join("broot");
    if cfg!(windows) {
        sandbox
            .smartopen()
            .args(["broot", "apply", "--target"])
            .arg(&broot_dir)
            .assert()
            .failure()
            .stderr(predicate::str::contains("Windows"));
        assert!(!broot_dir.exists(), "nothing may be written when refused");
        return;
    }
    sandbox
        .smartopen()
        .args(["broot", "apply", "--target"])
        .arg(&broot_dir)
        .assert()
        .success();
    assert!(broot_dir.join("smartopen.hjson").is_file());
    assert!(
        fs::read_to_string(broot_dir.join("conf.hjson"))
            .unwrap()
            .contains("smartopen.hjson")
    );
    sandbox
        .smartopen()
        .args(["broot", "check", "--target"])
        .arg(&broot_dir)
        .assert()
        .success();
}

#[test]
fn param_preset_with_print_renders_the_command() {
    let sandbox = Sandbox::new();
    sandbox.write_config("[[shortcut]]\nlabel = \"Checkout\"\nrun = \"git checkout {{branch}}\"\n");
    sandbox
        .smartopen()
        .args([
            "--command",
            "Checkout",
            "--param",
            "branch=main",
            "--print",
            "--no-history",
        ])
        .assert()
        .success()
        .stdout("git checkout main\n");
}

#[test]
fn when_gated_shortcut_appears_only_inside_a_cargo_project() {
    let sandbox = Sandbox::new();
    sandbox.write_config(
        "[[shortcut]]\nlabel = \"Cargo test\"\nrun = \"cargo test\"\n[shortcut.when]\ncwd_has = [\"Cargo.toml\"]\n",
    );
    let project = sandbox.path().join("proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::write(project.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    let elsewhere = sandbox.path().join("elsewhere");
    fs::create_dir_all(elsewhere.join(".git")).unwrap();

    sandbox
        .smartopen()
        .current_dir(&project)
        .args(["--command", "Cargo test", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("command: cargo test"));

    sandbox
        .smartopen()
        .current_dir(&elsewhere)
        .args(["--command", "Cargo test", "--dry-run"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("no command labeled")
                .or(predicate::str::contains("no shortcuts apply here")),
        );
}

#[test]
fn a_url_target_uses_the_url_association() {
    let sandbox = Sandbox::new();
    sandbox.write_config(
        "[[url]]\nschemes = [\"https\"]\nhosts = [\"github.com\"]\n[[url.command]]\nlabel = \"gh\"\nrun = \"gh browse {url} {host}\"\n",
    );
    sandbox
        .smartopen()
        .args(["--dry-run", "https://github.com/lariocpt/smartopen"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "command: gh browse https://github.com/lariocpt/smartopen github.com",
        ));
}

#[test]
fn several_targets_render_paths() {
    let sandbox = Sandbox::new();
    sandbox.write_config(&csv_config("xan cat rows {paths}"));
    let a = sandbox.path().join("a.csv");
    let b = sandbox.path().join("b.csv");
    fs::write(&a, "1\n").unwrap();
    fs::write(&b, "2\n").unwrap();
    let out = sandbox
        .smartopen()
        .arg("--dry-run")
        .arg(&a)
        .arg(&b)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains("a.csv") && printed.contains("b.csv"),
        "{printed}"
    );
}

#[test]
fn doctor_json_has_a_status_per_command_and_list_json_omits_empty_sections() {
    let sandbox = Sandbox::new();
    // A program only known at run time, spelled for this OS's shell: `${…}` is a
    // variable to `sh` and a missing program to `cmd`, which reads only `%VAR%`.
    let dynamic = if cfg!(windows) {
        "%EDITOR% --wait"
    } else {
        "${EDITOR:-nano}"
    };
    sandbox.write_config(&format!(
        "[[shortcut]]\nlabel = \"Dyn\"\nrun = \"{dynamic}\"\n"
    ));
    let out = sandbox
        .smartopen()
        .args(["config", "doctor", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(report["commands"][0]["status"], "dynamic");
    assert_eq!(report["problems"], 0);

    let out = sandbox
        .smartopen()
        .args(["config", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listing: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(listing["config"].get("extension").is_none(), "{listing}");
    assert_eq!(listing["config"]["shortcut"][0]["label"], "Dyn");
}

#[test]
fn tools_list_runs_and_names_the_navigators() {
    let sandbox = Sandbox::new();
    sandbox
        .smartopen()
        .args(["tools", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("yazi").and(predicate::str::contains("broot")));
}

#[test]
fn the_readme_deploy_example_runs_a_parameter_named_like_a_placeholder() {
    // `{{host}}` contains `{host}`; the renderer must read it as the parameter.
    let sandbox = Sandbox::new();
    sandbox.write_config(
        "[[shortcut]]\nlabel = \"Deploy\"\nrun = \"ssh {{host}} 'systemctl restart app'\"\n[shortcut.param.host]\ndefault = \"web-1\"\n",
    );
    // `web-1` needs no quoting on either shell; the rest is the template verbatim.
    let want = "ssh web-1 'systemctl restart app'\n";
    sandbox
        .smartopen()
        .args([
            "--command",
            "Deploy",
            "--param",
            "host=web-1",
            "--print",
            "--no-history",
        ])
        .assert()
        .success()
        .stdout(want);
}

#[test]
fn a_file_named_like_a_placeholder_is_quoted_once() {
    let sandbox = Sandbox::new();
    sandbox.write_config(&csv_config("echo {path}"));
    let dir = sandbox.path().join("{dir}");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("it {name}.csv");
    fs::write(&file, "a\n").unwrap();

    let out = sandbox
        .smartopen()
        .arg("--dry-run")
        .arg(&file)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let printed = String::from_utf8(out).unwrap();
    // One quoted value, braces intact, nothing substituted inside it.
    let quote = if cfg!(windows) { '"' } else { '\'' };
    assert!(
        printed.contains(&format!(
            "{{dir}}{}it {{name}}.csv{quote}",
            std::path::MAIN_SEPARATOR
        )),
        "{printed}"
    );
    assert_eq!(printed.matches(quote).count(), 2, "{printed}");
}

#[test]
fn json_without_list_or_doctor_is_refused_not_run() {
    // `smartopen --json file.csv` used to open the file with the flag ignored.
    let sandbox = Sandbox::new();
    sandbox.write_config(&csv_config("echo {path}"));
    let file = sandbox.path().join("a.csv");
    fs::write(&file, "a\n").unwrap();
    sandbox
        .smartopen()
        .arg("--json")
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--json goes with --list or --doctor",
        ));
}

#[test]
fn spec_without_rules_is_refused_rather_than_ignored() {
    let sandbox = Sandbox::new();
    let spec = sandbox
        .smartopen()
        .args(["yazi", "print-spec"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let spec_path = sandbox.path().join("spec.toml");
    fs::write(&spec_path, spec).unwrap();
    sandbox
        .smartopen()
        .args(["yazi", "print", "--spec"])
        .arg(&spec_path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--spec only applies with --rules"));
    sandbox
        .smartopen()
        .args(["yazi", "print", "--rules", "--spec"])
        .arg(&spec_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("[opener]"));
}

#[test]
fn a_command_that_names_only_the_first_of_several_targets_says_so() {
    let sandbox = Sandbox::new();
    sandbox.write_config(&csv_config("echo {path}"));
    let a = sandbox.path().join("a.csv");
    let b = sandbox.path().join("b.csv");
    fs::write(&a, "a\n").unwrap();
    fs::write(&b, "b\n").unwrap();
    sandbox
        .smartopen()
        .arg("--dry-run")
        .arg(&a)
        .arg(&b)
        .assert()
        .success()
        .stderr(predicate::str::contains("1 of 2 targets ignored"));

    // `{paths}` takes them all, and there is nothing to say.
    sandbox.write_config(&csv_config("echo {paths}"));
    sandbox
        .smartopen()
        .arg("--dry-run")
        .arg(&a)
        .arg(&b)
        .assert()
        .success()
        .stderr(predicate::str::contains("ignored").not());
}

#[test]
fn the_starter_config_answers_a_url_and_a_shebang_script() {
    // handlr opened a URL and an extensionless script with zero config; the starter
    // config used to answer both with "no matching commands".
    let sandbox = Sandbox::new();
    sandbox
        .smartopen()
        .args(["config", "init"])
        .assert()
        .success();
    sandbox
        .smartopen()
        .args(["--list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"url\""));
    #[cfg(unix)]
    {
        let script = sandbox.path().join("deploy");
        fs::write(&script, "#!/usr/bin/env python3\nprint('hi')\n").unwrap();
        sandbox
            .smartopen()
            .args(["--dry-run", "--command", "Edit"])
            .arg(&script)
            .assert()
            .success()
            .stdout(predicate::str::contains("deploy"));
    }
    let url = "https://github.com/lariocpt/smartopen";
    sandbox
        .smartopen()
        .args(["--dry-run", "--command", "Open in browser", url])
        .assert()
        .success()
        .stdout(predicate::str::contains(url));
}
