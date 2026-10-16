//! Session-only shell integration for bash, zsh, fish and PowerShell.
//!
//! Integration is installed through ephemeral temp files and command-line
//! arguments only; the user's own startup files are sourced first and are
//! never modified. The shells emit OSC 133 markers consumed by the block
//! engine:
//!
//! * `133;A` prompt start, `133;B` input start (end of prompt),
//! * `133;C` output start (command execution begins),
//! * `133;D;exit` command finished.
//!
//! Unsupported shells run raw with an explicit warning surfaced in the
//! snapshot, degrading gracefully (no blocks, plain terminal).

use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    /// Anything else: raw terminal fallback.
    Unsupported,
}

impl ShellKind {
    pub fn from_program(program: &Path) -> Self {
        let name = program
            .to_str()
            .map(|path| {
                path.rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(path)
                    .trim_end_matches(".exe")
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();
        match name.as_str() {
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            "pwsh" | "powershell" => ShellKind::PowerShell,
            _ => ShellKind::Unsupported,
        }
    }
}

#[derive(Debug)]
pub struct ShellLaunch {
    pub program: PathBuf,
    pub command: CommandBuilder,
    pub kind: ShellKind,
    /// Non-fatal integration warning (unsupported shell, etc).
    pub warning: Option<String>,
    /// Keeps the temp dir with integration files alive until dropped.
    pub _temp: Option<tempfile::TempDir>,
}

pub fn detect_shell(override_with: Option<&Path>) -> PathBuf {
    if let Some(path) = override_with {
        return path.to_path_buf();
    }
    #[cfg(unix)]
    {
        std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"))
    }
    #[cfg(windows)]
    {
        // Prefer PowerShell 7, then Windows PowerShell.
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if let Some(path) = windows_program_path(candidate) {
                return path;
            }
        }
        PathBuf::from("powershell.exe")
    }
}

#[cfg(windows)]
fn windows_program_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .or_else(|| {
            let system_root = std::env::var_os("SystemRoot")?;
            let mut fixed = PathBuf::from(system_root);
            fixed.push("System32");
            fixed.push("WindowsPowerShell");
            fixed.push("v1.0");
            fixed.push(program);
            fixed.is_file().then_some(fixed)
        })
}

/// `\033]133;` as it must appear inside the shell scripts (printf escapes).
const OSC: &str = "\\033]133;";
/// `\007` (BEL) terminator as it must appear inside the shell scripts.
const BEL: &str = "\\007";

pub fn build_launch(program: &Path, directory: &Path) -> ShellLaunch {
    match ShellKind::from_program(program) {
        ShellKind::Bash => bash_launch(program, directory),
        ShellKind::Zsh => zsh_launch(program, directory),
        ShellKind::Fish => fish_launch(program, directory),
        ShellKind::PowerShell => powershell_launch(program, directory),
        ShellKind::Unsupported => {
            let mut command = CommandBuilder::new(program);
            command.cwd(directory);
            let warning = format!(
                "shell '{}' is not supported for block integration; running a raw terminal",
                program.display()
            );
            ShellLaunch {
                program: program.to_path_buf(),
                command,
                kind: ShellKind::Unsupported,
                warning: Some(warning),
                _temp: None,
            }
        }
    }
}

fn bash_launch(program: &Path, directory: &Path) -> ShellLaunch {
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(err) => {
            return raw_fallback(
                program,
                directory,
                format!("bash integration skipped: {err}"),
            );
        }
    };
    let rcfile = temp.path().join("mud-bashrc.sh");
    let script = format!(
        r#"# mud-terminal session integration (bash)
# This file replaces .bashrc for this session only; the user's rc is sourced first.
case $- in *i*) ;; *) return 0 ;; esac
if [ -r "$HOME/.bashrc" ]; then . "$HOME/.bashrc"; fi
__mud_precmd() {{
    local __mud_ec=$?
    __mud_in_precmd=1
    printf '{OSC}D;%d{BEL}' "$__mud_ec"
    printf '{OSC}A{BEL}'
    case $PS1 in *133';'B*) ;; *) PS1="$PS1\[\033]133;B\007\]" ;; esac
    __mud_running=0
    __mud_in_precmd=0
}}
__mud_preexec() {{
    [ -n "$COMP_LINE" ] && return 0
    [ "${{__mud_in_precmd:-0}}" = 1 ] && return 0
    [ "${{__mud_running:-1}}" = 1 ] && return 0
    case "$BASH_COMMAND" in __mud_*) return 0 ;; esac
    case "${{FUNCNAME[1]}}" in __mud_*) return 0 ;; esac
    __mud_running=1
    local __mud_cmd
    __mud_cmd=$(HISTTIMEFORMAT= builtin history 1)
    __mud_cmd="${{__mud_cmd#*[0-9]  }}"
    __mud_cmd="${{__mud_cmd//$'\007'/}}"
    __mud_cmd="${{__mud_cmd//$'\033'/}}"
    printf '{OSC}E;%s{BEL}{OSC}C{BEL}' "$__mud_cmd"
}}
__mud_running=1
PROMPT_COMMAND="__mud_precmd${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}"
set -T
trap '__mud_preexec' DEBUG
"#
    );
    if let Err(err) = std::fs::write(&rcfile, script) {
        return raw_fallback(
            program,
            directory,
            format!("bash integration skipped: {err}"),
        );
    }
    let mut command = CommandBuilder::new(program);
    command.cwd(directory);
    command.arg("--rcfile");
    command.arg(&rcfile);
    command.arg("-i");
    ShellLaunch {
        program: program.to_path_buf(),
        command,
        kind: ShellKind::Bash,
        warning: None,
        _temp: Some(temp),
    }
}

fn zsh_launch(program: &Path, directory: &Path) -> ShellLaunch {
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(err) => {
            return raw_fallback(
                program,
                directory,
                format!("zsh integration skipped: {err}"),
            );
        }
    };
    // zsh reads all startup files from ZDOTDIR; we point it at our temp dir
    // and source the user's real files from $HOME explicitly.
    let zdot = temp.path().display().to_string();
    let zshenv = format!(
        r#"# mud-terminal session integration (zsh env)
[ -r "$HOME/.zshenv" ] && . "$HOME/.zshenv"
export ZDOTDIR="{zdot}"
"#
    );
    let zshrc = format!(
        r#"# mud-terminal session integration (zsh)
[[ -o interactive ]] || return 0
[ -r "$HOME/.zshrc" ] && . "$HOME/.zshrc"
__mud_precmd() {{
    local __mud_ec=$?
    printf '{OSC}D;%d{BEL}' "$__mud_ec"
    printf '{OSC}A{BEL}'
    case $PROMPT in *133';'B*) ;; *) __mud_b=$'\033]133;B\007'; PROMPT="$PROMPT%{{$__mud_b%}}" ;; esac
}}
__mud_preexec() {{
    local __mud_cmd="$1"
    __mud_cmd="${{__mud_cmd//$'\007'/}}"
    __mud_cmd="${{__mud_cmd//$'\033'/}}"
    printf '{OSC}E;%s{BEL}{OSC}C{BEL}' "$__mud_cmd"
}}
autoload -Uz add-zsh-hook
add-zsh-hook precmd __mud_precmd
add-zsh-hook preexec __mud_preexec
"#
    );
    if let Err(err) = std::fs::write(temp.path().join(".zshenv"), zshenv) {
        return raw_fallback(
            program,
            directory,
            format!("zsh integration skipped: {err}"),
        );
    }
    if let Err(err) = std::fs::write(temp.path().join(".zshrc"), zshrc) {
        return raw_fallback(
            program,
            directory,
            format!("zsh integration skipped: {err}"),
        );
    }
    let mut command = CommandBuilder::new(program);
    command.cwd(directory);
    command.env("ZDOTDIR", temp.path());
    command.arg("-i");
    ShellLaunch {
        program: program.to_path_buf(),
        command,
        kind: ShellKind::Zsh,
        warning: None,
        _temp: Some(temp),
    }
}

fn fish_launch(program: &Path, directory: &Path) -> ShellLaunch {
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(err) => {
            return raw_fallback(
                program,
                directory,
                format!("fish integration skipped: {err}"),
            );
        }
    };
    let script_path = temp.path().join("mud-fish.fish");
    let script = format!(
        r#"# mud-terminal session integration (fish)
function __mud_precmd --on-event fish_prompt
    set -l __mud_ec $status
    printf '{OSC}D;%d{BEL}' $__mud_ec
    printf '{OSC}A{BEL}'
end
function __mud_preexec --on-event fish_preexec
    printf '{OSC}C{BEL}'
end
functions --copy fish_prompt __mud_fish_prompt
function fish_prompt
    __mud_fish_prompt
    printf '{OSC}B{BEL}'
end
"#
    );
    if let Err(err) = std::fs::write(&script_path, script) {
        return raw_fallback(
            program,
            directory,
            format!("fish integration skipped: {err}"),
        );
    }
    let mut command = CommandBuilder::new(program);
    command.cwd(directory);
    // -C runs after the user's config without redirecting config lookup for child TUI apps.
    command.arg("-i");
    command.arg("-C");
    command.arg(format!(
        "source '{}'",
        script_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
    ));
    ShellLaunch {
        program: program.to_path_buf(),
        command,
        kind: ShellKind::Fish,
        warning: None,
        _temp: Some(temp),
    }
}

fn powershell_launch(program: &Path, directory: &Path) -> ShellLaunch {
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(err) => {
            return raw_fallback(
                program,
                directory,
                format!("powershell integration skipped: {err}"),
            );
        }
    };
    let script_path = temp.path().join("mud-prompt.ps1");
    // PowerShell has no native pre-exec hook; PSConsoleHostReadLine is invoked
    // right after <Enter> and immediately before the command executes, which
    // is exactly where the C marker belongs. The prompt wraps the user's own.
    let script = r#"
$global:__mudEsc = [char]27
$global:__mudBel = [char]7
$global:__mudOrigPrompt = $function:prompt
function global:prompt {
    $ok = $?
    $ec = 0
    if (-not $ok) {
        $ec = 1
        if ($null -ne $global:LASTEXITCODE -and $global:LASTEXITCODE -ne 0) { $ec = $global:LASTEXITCODE }
    }
    [Console]::Write("$global:__mudEsc]133;D;$ec$global:__mudBel")
    [Console]::Write("$global:__mudEsc]133;A$global:__mudBel")
    $text = $global:__mudOrigPrompt.Invoke()
    [Console]::Out.Write($text)
    [Console]::Write("$global:__mudEsc]133;B$global:__mudBel")
    return ""
}
function global:PSConsoleHostReadLine {
    $line = [Microsoft.PowerShell.PSConsoleReadLine]::ReadLine($Host.Runspace, $ExecutionContext)
    [Console]::Write("$global:__mudEsc]133;C$global:__mudBel")
    $line
}
"#;
    if let Err(err) = std::fs::write(&script_path, script) {
        return raw_fallback(
            program,
            directory,
            format!("powershell integration skipped: {err}"),
        );
    }
    let mut command = CommandBuilder::new(program);
    command.cwd(directory);
    command.arg("-NoLogo");
    command.arg("-NoExit");
    command.arg("-ExecutionPolicy");
    command.arg("Bypass");
    command.arg("-File");
    command.arg(&script_path);
    ShellLaunch {
        program: program.to_path_buf(),
        command,
        kind: ShellKind::PowerShell,
        warning: None,
        _temp: Some(temp),
    }
}

fn raw_fallback(program: &Path, directory: &Path, warning: String) -> ShellLaunch {
    let mut command = CommandBuilder::new(program);
    command.cwd(directory);
    ShellLaunch {
        program: program.to_path_buf(),
        command,
        kind: ShellKind::Unsupported,
        warning: Some(warning),
        _temp: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_detection() {
        assert_eq!(
            ShellKind::from_program(Path::new("/bin/bash")),
            ShellKind::Bash
        );
        assert_eq!(
            ShellKind::from_program(Path::new("/usr/bin/zsh")),
            ShellKind::Zsh
        );
        assert_eq!(
            ShellKind::from_program(Path::new("/opt/fish/bin/fish")),
            ShellKind::Fish
        );
        assert_eq!(
            ShellKind::from_program(Path::new("pwsh.exe")),
            ShellKind::PowerShell
        );
        assert_eq!(
            ShellKind::from_program(Path::new(
                "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"
            )),
            ShellKind::PowerShell
        );
        assert_eq!(
            ShellKind::from_program(Path::new("/bin/sh")),
            ShellKind::Unsupported
        );
        assert_eq!(
            ShellKind::from_program(Path::new("/bin/tcsh")),
            ShellKind::Unsupported
        );
    }

    #[test]
    fn bash_rcfile_sources_user_rc_and_installs_hooks() {
        let launch = bash_launch(Path::new("/bin/bash"), Path::new("/tmp"));
        assert_eq!(launch.kind, ShellKind::Bash);
        assert!(launch.warning.is_none());
        // test-only setup: the launcher stores its TempDir fixture in _temp
        let temp = launch
            ._temp
            .as_ref()
            .map(|t| t.path())
            .expect("bash launch must carry its temp rcfile dir");
        let content = std::fs::read_to_string(temp.join("mud-bashrc.sh")).unwrap_or_default();
        assert!(content.contains(". \"$HOME/.bashrc\""));
        assert!(content.contains("__mud_precmd"));
        assert!(content.contains("133;D;%d"));
        assert!(content.contains("133;A"));
        assert!(content.contains("133;C"));
        assert!(content.contains("133;B"));
        // The DEBUG trap must not fire for our own precmd emissions.
        assert!(content.contains("__mud_in_precmd"));
        assert!(content.contains("trap '__mud_preexec' DEBUG"));
    }

    #[test]
    fn zsh_launch_wires_zdotdir() {
        let launch = zsh_launch(Path::new("/bin/zsh"), Path::new("/tmp"));
        // test-only setup: the launcher stores its TempDir fixture in _temp
        let temp = launch
            ._temp
            .as_ref()
            .map(|t| t.path())
            .expect("zsh launch must carry its temp ZDOTDIR dir");
        assert!(temp.join(".zshrc").is_file());
        assert!(temp.join(".zshenv").is_file());
        let rc = std::fs::read_to_string(temp.join(".zshrc")).unwrap_or_default();
        assert!(rc.contains("add-zsh-hook preexec __mud_preexec"));
        assert!(rc.contains(". \"$HOME/.zshrc\""));
        assert!(rc.contains("133;B"));
        let env = std::fs::read_to_string(temp.join(".zshenv")).unwrap_or_default();
        assert!(env.contains("export ZDOTDIR="));
    }

    #[test]
    fn fish_launch_sources_hooks_after_user_config() {
        let launch = fish_launch(Path::new("/opt/fish/bin/fish"), Path::new("/tmp"));
        // test-only setup: the launcher stores its TempDir fixture in _temp
        let temp = launch
            ._temp
            .as_ref()
            .map(|t| t.path())
            .expect("fish launch must carry its temp script dir");
        assert!(temp.join("mud-fish.fish").is_file());
        let config = std::fs::read_to_string(temp.join("mud-fish.fish")).unwrap_or_default();
        assert!(config.contains("fish_preexec"));
        assert!(config.contains("133;B"));
    }

    #[test]
    fn unsupported_shell_runs_raw_with_warning() {
        let launch = build_launch(Path::new("/bin/sh"), Path::new("/tmp"));
        assert_eq!(launch.kind, ShellKind::Unsupported);
        assert!(
            launch
                .warning
                .as_deref()
                .unwrap_or("")
                .contains("not supported")
        );
        assert!(launch._temp.is_none());
    }

    #[test]
    fn powershell_script_wraps_prompt() {
        let launch = powershell_launch(Path::new("pwsh.exe"), Path::new("/tmp"));
        // test-only setup: the launcher stores its TempDir fixture in _temp
        let temp = launch
            ._temp
            .as_ref()
            .map(|t| t.path())
            .expect("pwsh launch must carry its temp script dir");
        let script = std::fs::read_to_string(temp.join("mud-prompt.ps1")).unwrap_or_default();
        assert!(script.contains("PSConsoleHostReadLine"));
        assert!(script.contains("133;C"));
        assert!(script.contains("__mudOrigPrompt"));
    }
}
