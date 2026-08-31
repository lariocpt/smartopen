use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::CommandEntry;
use crate::params::is_name;
use crate::shell::Shell;
use crate::target::Target;
use crate::terminal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    pub command: String,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAvailability {
    Found { executable: String, path: PathBuf },
    Missing { executable: String },
    Dynamic { reason: String },
    Empty,
}

impl CommandAvailability {
    pub fn summary(&self) -> String {
        match self {
            Self::Found { executable, path } => {
                format!("{executable}: found at {}", path.display())
            }
            Self::Missing { executable } => format!("{executable}: missing from PATH"),
            Self::Dynamic { reason } => format!("dynamic shell command: {reason}"),
            Self::Empty => "empty command".to_string(),
        }
    }
}

/// Every placeholder a command may use. `{path}`… come from the first target; `{paths}`
/// is every target; the URL three render a file as `file://…` / `file` / empty.
pub const PLACEHOLDERS: &[&str] = &[
    "{path}", "{paths}", "{dir}", "{name}", "{stem}", "{ext}", "{url}", "{scheme}", "{host}",
];

/// Run the command and return the exit code the child reported, so the launcher exits the
/// way the launched program did. A detached launch reports 0: there is nothing to wait
/// for. Only a failure to start at all is an error.
pub fn run_command(
    command: &CommandEntry,
    targets: &[Target],
    params: Option<&BTreeMap<String, String>>,
) -> Result<i32> {
    let mut plan = plan_command_with(command, targets, params)?;

    // A new terminal window is a GUI launch: wrap the line for the terminal program and
    // let go of it, the same as `detach`.
    if command.terminal {
        plan.command = terminal::wrap(&plan.command, plan.cwd.as_deref())?;
    }

    if command.detach || command.terminal {
        spawn_detached(&plan, command)?;
        return Ok(0);
    }

    let mut process = Shell::current().command(&plan.command);
    process
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some(cwd) = &plan.cwd {
        process.current_dir(cwd);
    }

    let status = process
        .status()
        .with_context(|| format!("failed to run command '{}'", command.label))?;

    // No code means a signal killed it (Unix); 128+n is what shells report for that.
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + signal_number(&status)))
}

#[cfg(unix)]
fn signal_number(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.signal().unwrap_or(1)
}

#[cfg(not(unix))]
fn signal_number(_status: &std::process::ExitStatus) -> i32 {
    1
}

/// Launch a command fully in the background (GUI apps): no wait, no inherited stdio, so the
/// menu returns immediately and a non-zero exit is not surfaced — the opener's `orphan`.
///
/// "Fully" means its own process group (Unix) or a detached console (Windows). Without
/// that, closing the terminal that ran the menu takes the launched app down with it.
fn spawn_detached(plan: &ExecutionPlan, command: &CommandEntry) -> Result<()> {
    let mut process = Shell::current().command(&plan.command);
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(cwd) = &plan.cwd {
        process.current_dir(cwd);
    }

    detach_from_terminal(&mut process);

    process
        .spawn()
        .with_context(|| format!("failed to launch '{}'", command.label))?;
    Ok(())
}

#[cfg(unix)]
fn detach_from_terminal(process: &mut Command) {
    use std::os::unix::process::CommandExt;
    // A new process group: SIGHUP for the terminal's group no longer reaches the child.
    process.process_group(0);
}

#[cfg(windows)]
fn detach_from_terminal(process: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    process.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach_from_terminal(_process: &mut Command) {}

pub fn plan_command(command: &CommandEntry, targets: &[Target]) -> Result<ExecutionPlan> {
    plan_command_with(command, targets, None)
}

/// [`plan_command`] with the `{{parameter}}` values filled in. `None` leaves `{{name}}`
/// verbatim, which is what a preview wants; `Some` must answer every name.
pub fn plan_command_with(
    command: &CommandEntry,
    targets: &[Target],
    params: Option<&BTreeMap<String, String>>,
) -> Result<ExecutionPlan> {
    let command_line = render_command_with(Shell::current(), command, targets, params)?;
    let cwd = plan_cwd(command, targets, params)?;
    Ok(ExecutionPlan {
        command: command_line,
        cwd,
    })
}

/// The working directory, with placeholders and parameters substituted UNQUOTED — it is a
/// path handed to `Command::current_dir`, not shell text — and then `~`/`$VAR` expanded.
/// This is what makes the wizard's `cwd = "{path}"` on a folder command mean the folder.
pub fn plan_cwd(
    command: &CommandEntry,
    targets: &[Target],
    params: Option<&BTreeMap<String, String>>,
) -> Result<Option<PathBuf>> {
    command
        .cwd
        .as_deref()
        .map(|cwd| {
            expand_path(&render_template(
                cwd,
                targets,
                params,
                None,
                &command.label,
            )?)
        })
        .transpose()
        .with_context(|| format!("failed to prepare cwd for command '{}'", command.label))
}

pub fn command_availability(command_line: &str) -> CommandAvailability {
    match first_executable(command_line) {
        ExecutableHint::Name(executable) => match find_executable(&executable) {
            Some(path) => CommandAvailability::Found { executable, path },
            None => CommandAvailability::Missing { executable },
        },
        ExecutableHint::Dynamic(reason) => CommandAvailability::Dynamic { reason },
        ExecutableHint::Empty => CommandAvailability::Empty,
    }
}

/// Substitute the target placeholders and the `{{parameters}}`, each quoted for `shell`.
/// Takes the shell explicitly so both quoting rules are testable anywhere.
pub fn render_command_with(
    shell: Shell,
    command: &CommandEntry,
    targets: &[Target],
    params: Option<&BTreeMap<String, String>>,
) -> Result<String> {
    render_template(&command.run, targets, params, Some(shell), &command.label)
}

/// One left-to-right pass over `template`. `{{name}}` is a parameter, `{token}` a target
/// placeholder, and any other brace — `${EDITOR:-nano}`, a stray `{` — is copied as it is.
/// Substituted text is never rescanned: a file named `{dir}` or a parameter answered with
/// `{path}` is quoted once and stays inside its quotes. The chain of `str::replace` calls
/// this replaces rescanned every inserted value, so a filename containing a later
/// placeholder closed its own quote — a review found it.
///
/// `quote = None` inserts raw values (a working directory is a path, not shell text).
/// `params = None` leaves `{{name}}` verbatim (a preview); `Some` must answer every name.
fn render_template(
    template: &str,
    targets: &[Target],
    params: Option<&BTreeMap<String, String>>,
    quote: Option<Shell>,
    label: &str,
) -> Result<String> {
    let emit = |value: &str| -> Result<String> {
        match quote {
            Some(shell) => shell
                .quote(value)
                .with_context(|| format!("cannot render command '{label}'")),
            None => Ok(value.to_string()),
        }
    };

    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];

        // `{{name}}`: a parameter.
        if let Some(inner) = after.strip_prefix("{{")
            && let Some(end) = inner.find("}}")
            && is_name(inner[..end].trim())
        {
            let name = inner[..end].trim();
            let consumed = 2 + end + 2;
            match params {
                None => out.push_str(&after[..consumed]),
                Some(values) => {
                    let value = values
                        .get(name)
                        .with_context(|| format!("parameter {{{{{name}}}}} has no value"))?;
                    out.push_str(&emit(value)?);
                }
            }
            rest = &after[consumed..];
            continue;
        }

        // `{token}`: a target placeholder.
        if let Some(end) = after.find('}')
            && PLACEHOLDERS.contains(&&after[..=end])
        {
            let token = &after[..=end];
            let Some(first) = targets.first() else {
                bail!(
                    "shortcut '{label}' uses a target placeholder, but no path or URL was provided"
                );
            };
            let value = match token {
                "{paths}" => {
                    let mut all = Vec::with_capacity(targets.len());
                    for target in targets {
                        all.push(emit(&target.path.display().to_string())?);
                    }
                    all.join(" ")
                }
                "{path}" => emit(&first.path.display().to_string())?,
                "{dir}" => emit(&first.dir.display().to_string())?,
                "{name}" => emit(&first.name)?,
                "{stem}" => emit(&first.stem)?,
                "{ext}" => emit(&first.ext)?,
                "{url}" => emit(&first.as_url_string())?,
                "{scheme}" => emit(first.url.as_ref().map_or("file", |u| u.scheme.as_str()))?,
                "{host}" => emit(first.url.as_ref().map_or("", |u| u.host.as_str()))?,
                _ => unreachable!("every PLACEHOLDERS entry is matched above"),
            };
            out.push_str(&value);
            rest = &after[end + 1..];
            continue;
        }

        // Neither: a literal brace.
        out.push('{');
        rest = &after[1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn expand_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path)
        .with_context(|| format!("failed to expand path '{path}'"))?
        .into_owned();
    Ok(PathBuf::from(expanded))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutableHint {
    Name(String),
    Dynamic(String),
    Empty,
}

/// cmd.exe's own commands: no file on `PATH` answers for them, so they are neither missing
/// nor found. The Windows starter config leans on `start` and `cd`.
const CMD_BUILTINS: &[&str] = &[
    "assoc", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir", "echo",
    "endlocal", "erase", "exit", "for", "ftype", "goto", "if", "md", "mkdir", "mklink", "move",
    "path", "pause", "popd", "prompt", "pushd", "rd", "rem", "ren", "rename", "rmdir", "set",
    "setlocal", "shift", "start", "time", "title", "type", "ver", "verify", "vol",
];

fn first_executable(command_line: &str) -> ExecutableHint {
    first_executable_for(Shell::current(), command_line)
}

/// The first word a shell would execute, read with THAT shell's rules: cmd has no single
/// quotes and no backslash escapes, so `C:\Tools\x.exe` is one word there and three
/// characters short of one under sh's rules; `VAR=x prog` and `sudo prog` are sh idioms.
fn first_executable_for(shell: Shell, command_line: &str) -> ExecutableHint {
    let mut offset = 0;
    let mut allow_assignment = true;

    while let Some((word, next_offset)) = next_shell_word(shell, command_line, offset) {
        offset = next_offset;

        if word.is_empty() {
            continue;
        }

        // `run = "{path}"`: the target is the program, and a `{{param}}` is whatever
        // gets typed. Nothing to look up until it is substituted.
        if word.starts_with('{') {
            return ExecutableHint::Dynamic(format!("{word} is substituted at run time"));
        }

        if starts_with_shell_expansion(shell, &word) {
            return ExecutableHint::Dynamic(format!("starts with {word}"));
        }

        if shell == Shell::Posix && allow_assignment && is_env_assignment(&word) {
            continue;
        }

        allow_assignment = false;

        if shell == Shell::Posix && matches!(word.as_str(), "sudo" | "doas" | "command" | "exec") {
            continue;
        }

        if shell == Shell::Cmd && CMD_BUILTINS.contains(&word.to_ascii_lowercase().as_str()) {
            return ExecutableHint::Dynamic(format!("{word} is a cmd builtin"));
        }

        if contains_shell_expansion(shell, &word) {
            return ExecutableHint::Dynamic(format!("executable contains expansion: {word}"));
        }

        return ExecutableHint::Name(word);
    }

    ExecutableHint::Empty
}

fn next_shell_word(shell: Shell, command_line: &str, start: usize) -> Option<(String, usize)> {
    let mut index = start;
    let mut word = String::new();
    let mut chars = command_line[start..].char_indices().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some((relative_index, ch)) = chars.peek().copied() {
        index = start + relative_index;
        if !in_single_quote && !in_double_quote && ch.is_whitespace() {
            chars.next();
            continue;
        }
        break;
    }

    while let Some((relative_index, ch)) = chars.next() {
        index = start + relative_index + ch.len_utf8();

        if !in_single_quote && !in_double_quote && ch.is_whitespace() {
            break;
        }

        if !in_single_quote && !in_double_quote && matches!(ch, '|' | ';' | '&' | '(') {
            break;
        }

        match ch {
            '\'' if shell == Shell::Posix && !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '\\' if shell == Shell::Posix && !in_single_quote => {
                if let Some((_, escaped)) = chars.next() {
                    word.push(escaped);
                    index += escaped.len_utf8();
                }
            }
            _ => word.push(ch),
        }
    }

    if word.is_empty() && index >= command_line.len() {
        None
    } else if word.is_empty() {
        next_shell_word(shell, command_line, index)
    } else {
        Some((word, index))
    }
}

fn starts_with_shell_expansion(shell: Shell, word: &str) -> bool {
    match shell {
        Shell::Posix => word.starts_with('$') || word.starts_with('`'),
        Shell::Cmd => is_cmd_variable(word),
    }
}

fn contains_shell_expansion(shell: Shell, word: &str) -> bool {
    match shell {
        Shell::Posix => word.contains('$') || word.contains('`'),
        Shell::Cmd => word.contains('%'),
    }
}

/// `%EDITOR%`-style cmd.exe expansion.
fn is_cmd_variable(word: &str) -> bool {
    word.starts_with('%') && word.len() > 2 && word.ends_with('%')
}

fn is_env_assignment(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };

    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub fn find_executable(executable: &str) -> Option<PathBuf> {
    let executable_path = Path::new(executable);
    if executable_path.components().count() > 1 {
        return executable_path
            .exists()
            .then(|| executable_path.to_path_buf());
    }

    let path = env::var_os("PATH")?;
    let names = executable_names(executable);
    env::split_paths(&path)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

/// The file names a bare command may resolve to. On Windows `xan` is `xan.exe` (or any
/// other `PATHEXT` extension); everywhere else the name is the file name.
fn executable_names(executable: &str) -> Vec<String> {
    if !cfg!(windows) || Path::new(executable).extension().is_some() {
        return vec![executable.to_string()];
    }

    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut names = vec![executable.to_string()];
    names.extend(
        pathext
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(|ext| format!("{executable}{}", ext.to_lowercase())),
    );
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(run: &str) -> CommandEntry {
        CommandEntry {
            label: "Open".to_string(),
            run: run.to_string(),
            ..CommandEntry::default()
        }
    }

    fn render(shell: Shell, run: &str, targets: &[Target]) -> Result<String> {
        render_command_with(shell, &command(run), targets, None)
    }

    #[test]
    fn posix_render_single_quotes_path_placeholders() {
        let target = Target::fake_file("/tmp/project dir/hello world.rs");

        let rendered = render(Shell::Posix, "vim {path}", &[target]).unwrap();

        assert_eq!(rendered, "vim '/tmp/project dir/hello world.rs'");
    }

    #[test]
    fn cmd_render_double_quotes_path_placeholders() {
        // Built by hand: a Unix `Path` does not split on backslashes, and this test runs
        // on every OS.
        let mut target = Target::fake_file("/placeholder.rs");
        target.path = PathBuf::from(r"C:\project dir\hello world.rs");
        target.dir = PathBuf::from(r"C:\project dir");
        target.name = "hello world.rs".to_string();

        let rendered = render(Shell::Cmd, "micro {path} {dir} {name}", &[target]).unwrap();

        assert_eq!(
            rendered,
            r#"micro "C:\project dir\hello world.rs" "C:\project dir" "hello world.rs""#
        );
    }

    #[test]
    fn cmd_render_refuses_a_percent_in_the_path() {
        let target = Target::fake_file(r"C:\100%\a.rs");

        let error = render(Shell::Cmd, "micro {path}", &[target]).expect_err("% cannot be quoted");

        assert!(error.to_string().contains("cannot render command 'Open'"));
    }

    #[test]
    fn render_shortcut_rejects_target_placeholders() {
        assert!(render(Shell::Posix, "vim {path}", &[]).is_err());
        assert!(render(Shell::Posix, "open {url}", &[]).is_err());
        assert_eq!(
            render(Shell::Posix, "cargo test", &[]).unwrap(),
            "cargo test"
        );
    }

    #[test]
    fn render_command_preserves_shell_default_expansion() {
        let target = Target::fake_file("/tmp/file.rs");

        let rendered = render(Shell::Posix, "${EDITOR:-nano} {path}", &[target]).unwrap();

        assert_eq!(rendered, "${EDITOR:-nano} /tmp/file.rs");
    }

    #[test]
    fn paths_placeholder_renders_every_target_and_path_the_first() {
        let targets = [
            Target::fake_file("/tmp/a b.csv"),
            Target::fake_file("/tmp/c.csv"),
        ];

        let rendered = render(Shell::Posix, "xan cat {paths} --first {path}", &targets).unwrap();

        assert_eq!(
            rendered,
            "xan cat '/tmp/a b.csv' /tmp/c.csv --first '/tmp/a b.csv'"
        );
    }

    #[test]
    fn url_placeholders_come_from_the_url_and_degrade_for_files() {
        let url = Target::from_arg("https://Example.com/x/report.pdf").unwrap();
        let rendered = render(Shell::Posix, "open {url} {scheme} {host} {name}", &[url]).unwrap();
        assert_eq!(
            rendered,
            "open https://Example.com/x/report.pdf https example.com report.pdf"
        );

        let file = Target::fake_file("/tmp/a.txt");
        let rendered = render(Shell::Posix, "open {url} {scheme} {host}", &[file]).unwrap();
        assert_eq!(rendered, "open file:///tmp/a.txt file ''");
    }

    #[test]
    fn a_placeholder_in_a_file_name_is_quoted_once_and_never_rescanned() {
        // The old replace chain turned the `{name}` inside the already-quoted path into
        // another quoted value, closing the quote and handing the rest to the shell bare.
        let target = Target::fake_file("/tmp/{dir}/it {name}.txt");

        let rendered = render(Shell::Posix, "cat {path}", &[target]).unwrap();

        assert_eq!(rendered, "cat '/tmp/{dir}/it {name}.txt'");
    }

    #[test]
    fn parameters_and_placeholders_are_one_pass_and_a_value_may_look_like_either() {
        let target = Target::fake_file("/tmp/a.txt");
        let mut values = BTreeMap::new();
        values.insert("host".to_string(), "web-1; rm {path}".to_string());

        let rendered = render_command_with(
            Shell::Posix,
            &command("ssh {{host}} cat {path} {{ host }}"),
            &[target],
            Some(&values),
        )
        .unwrap();

        assert_eq!(
            rendered,
            "ssh 'web-1; rm {path}' cat /tmp/a.txt 'web-1; rm {path}'"
        );
    }

    #[test]
    fn a_parameter_named_like_a_placeholder_is_a_parameter() {
        // `{{host}}` must not be read as `{host}` — the README's own Deploy example.
        let mut values = BTreeMap::new();
        values.insert("host".to_string(), "box".to_string());
        assert_eq!(
            render_command_with(
                Shell::Posix,
                &command("ssh {{host}} true"),
                &[],
                Some(&values)
            )
            .unwrap(),
            "ssh box true"
        );
        // A preview (no values) keeps it verbatim and does not ask for a target either.
        assert_eq!(
            render(Shell::Posix, "ssh {{host}} true", &[]).unwrap(),
            "ssh {{host}} true"
        );
        assert!(
            render_command_with(
                Shell::Posix,
                &command("ssh {{host}}"),
                &[],
                Some(&BTreeMap::new())
            )
            .is_err()
        );
    }

    #[test]
    fn braces_that_are_neither_are_left_alone() {
        let target = Target::fake_file("/tmp/a.txt");
        assert_eq!(
            render(
                Shell::Posix,
                "${EDITOR:-nano} {path} {nope} {{ }}",
                &[target]
            )
            .unwrap(),
            "${EDITOR:-nano} /tmp/a.txt {nope} {{ }}"
        );
    }

    #[test]
    fn cwd_takes_placeholders_raw() {
        let target = Target::fake_dir("/tmp/my proj");
        let command = CommandEntry {
            label: "T".to_string(),
            run: "true".to_string(),
            cwd: Some("{path}".to_string()),
            ..CommandEntry::default()
        };

        let plan = plan_command(&command, &[target]).unwrap();

        assert_eq!(plan.cwd, Some(PathBuf::from("/tmp/my proj")));
        assert_eq!(plan.command, "true");
    }

    #[test]
    fn plan_command_expands_cwd() {
        let command = CommandEntry {
            label: "Build".to_string(),
            run: "cargo build".to_string(),
            cwd: Some(".".to_string()),
            ..CommandEntry::default()
        };

        let plan = plan_command(&command, &[]).expect("command should plan");

        assert_eq!(plan.command, "cargo build");
        assert!(plan.cwd.is_some());
    }

    #[test]
    fn first_executable_finds_simple_binary() {
        assert_eq!(
            first_executable_for(Shell::Posix, "csvi {path}"),
            ExecutableHint::Name("csvi".to_string())
        );
    }

    #[test]
    fn first_executable_skips_env_assignments_and_wrappers() {
        assert_eq!(
            first_executable_for(Shell::Posix, "FOO=bar sudo xan view {path}"),
            ExecutableHint::Name("xan".to_string())
        );
    }

    #[test]
    fn first_executable_marks_shell_expansion_as_dynamic() {
        assert_eq!(
            first_executable_for(Shell::Posix, "${EDITOR:-nano} {path}"),
            ExecutableHint::Dynamic("starts with ${EDITOR:-nano}".to_string())
        );
    }

    #[test]
    fn cmd_words_keep_their_backslashes_and_quoted_spaces() {
        assert_eq!(
            first_executable_for(Shell::Cmd, r#""C:\Program Files\x\micro.exe" {path}"#),
            ExecutableHint::Name(r"C:\Program Files\x\micro.exe".to_string())
        );
        assert_eq!(
            first_executable_for(Shell::Cmd, r"C:\Tools\xan.exe view {path}"),
            ExecutableHint::Name(r"C:\Tools\xan.exe".to_string())
        );
        // sh's rules on the same line would eat every backslash.
        assert_eq!(
            first_executable_for(Shell::Posix, r"C:\Tools\xan.exe view"),
            ExecutableHint::Name("C:Toolsxan.exe".to_string())
        );
    }

    #[test]
    fn cmd_builtins_and_variables_are_dynamic_not_missing() {
        assert_eq!(
            first_executable_for(Shell::Cmd, "start \"\" {path}"),
            ExecutableHint::Dynamic("start is a cmd builtin".to_string())
        );
        assert_eq!(
            first_executable_for(Shell::Cmd, "cd /d {path} && gitui"),
            ExecutableHint::Dynamic("cd is a cmd builtin".to_string())
        );
        assert_eq!(
            first_executable_for(Shell::Cmd, "%EDITOR% {path}"),
            ExecutableHint::Dynamic("starts with %EDITOR%".to_string())
        );
        // Not sh idioms: on cmd these are the program.
        assert_eq!(
            first_executable_for(Shell::Cmd, "FOO=bar prog"),
            ExecutableHint::Name("FOO=bar".to_string())
        );
    }

    #[test]
    fn command_availability_reports_missing_binary() {
        let availability = command_availability("definitely-not-installed-opn-test-binary {path}");

        assert_eq!(
            availability,
            CommandAvailability::Missing {
                executable: "definitely-not-installed-opn-test-binary".to_string()
            }
        );
    }

    #[test]
    fn executable_names_add_pathext_only_on_windows() {
        let names = executable_names("xan");
        if cfg!(windows) {
            assert!(names.iter().any(|name| name == "xan.exe"), "{names:?}");
        } else {
            assert_eq!(names, vec!["xan".to_string()]);
        }
        assert_eq!(executable_names("xan.exe"), vec!["xan.exe".to_string()]);
    }
}
