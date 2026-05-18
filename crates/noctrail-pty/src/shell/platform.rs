use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use crate::{PtyCommand, ShellSource};

pub(crate) fn detect_shell_program<I, F>(env_vars: I, split_paths: F) -> (OsString, ShellSource)
where
    I: IntoIterator<Item = (OsString, OsString)>,
    F: Fn(&OsStr) -> env::SplitPaths,
{
    let mut comspec = None;
    let mut path_value = None;
    let mut system_root = None;
    let mut program_files = None;
    let mut program_w6432 = None;

    for (key, value) in env_vars {
        let key_text = key.to_string_lossy();
        if key_text.eq_ignore_ascii_case("COMSPEC") && !value.is_empty() {
            comspec = Some(value);
        } else if key_text.eq_ignore_ascii_case("PATH") {
            path_value = Some(value);
        } else if key_text.eq_ignore_ascii_case("SYSTEMROOT") && !value.is_empty() {
            system_root = Some(value);
        } else if key_text.eq_ignore_ascii_case("PROGRAMFILES") && !value.is_empty() {
            program_files = Some(value);
        } else if key_text.eq_ignore_ascii_case("ProgramW6432") && !value.is_empty() {
            program_w6432 = Some(value);
        }
    }

    for (program, source) in [
        ("pwsh.exe", ShellSource::PathPwsh),
        ("powershell.exe", ShellSource::PathPowerShell),
    ] {
        if program_exists_on_path(path_value.as_deref(), program, &split_paths) {
            return (OsString::from(program), source);
        }
    }

    for root in [program_w6432.as_deref(), program_files.as_deref()]
        .into_iter()
        .flatten()
    {
        let candidate = PathBuf::from(root)
            .join("PowerShell")
            .join("7")
            .join("pwsh.exe");
        if path_is_executable(&candidate) {
            return (candidate.into_os_string(), ShellSource::PathPwsh);
        }
    }

    if let Some(root) = system_root {
        let candidate = PathBuf::from(root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        if path_is_executable(&candidate) {
            return (candidate.into_os_string(), ShellSource::PathPowerShell);
        }
    }

    if let Some(program) = comspec {
        return (program, ShellSource::EnvComSpec);
    }

    if program_exists_on_path(path_value.as_deref(), "wsl.exe", &split_paths) {
        return (OsString::from("wsl.exe"), ShellSource::PathWsl);
    }

    (OsString::from("cmd.exe"), ShellSource::FallbackCmd)
}

pub(crate) fn configure_shell_command(
    command: &mut PtyCommand,
    program: &OsStr,
    source: ShellSource,
) {
    if !matches!(source, ShellSource::PathPwsh | ShellSource::PathPowerShell) {
        return;
    }

    let shell = Path::new(program)
        .file_name()
        .unwrap_or(program)
        .to_string_lossy()
        .to_ascii_lowercase();
    if !matches!(
        shell.as_str(),
        "pwsh.exe" | "pwsh" | "powershell.exe" | "powershell"
    ) {
        return;
    }

    command
        .arg("-NoLogo")
        .arg("-NoExit")
        .arg("-Command")
        .arg(render_powershell_hook());
}

fn program_exists_on_path<F>(path_value: Option<&OsStr>, program: &str, split_paths: &F) -> bool
where
    F: Fn(&OsStr) -> env::SplitPaths,
{
    let Some(path_value) = path_value else {
        return false;
    };

    split_paths(path_value).any(|dir| path_is_executable(&dir.join(program)))
}

fn path_is_executable(path: &Path) -> bool {
    path.is_file()
}

fn render_powershell_hook() -> String {
    r#"function global:__NoctrailEmit([string]$Payload) {
    [Console]::Out.Write("$([char]27)]1337;Noctrail;$Payload$([char]7)")
}

$global:__NoctrailOriginalPrompt = $function:prompt

function global:prompt {
    $exitCode = if ($null -ne $global:LASTEXITCODE) { $global:LASTEXITCODE } else { 0 }
    __NoctrailEmit "Prompt"
    __NoctrailEmit ("Cwd;" + (Get-Location).Path)
    if ($global:__NoctrailCommandActive) {
        $durationMs = [int]((Get-Date) - $global:__NoctrailCommandStart).TotalMilliseconds
        __NoctrailEmit ("ExitCode;" + $exitCode)
        __NoctrailEmit ("DurationMs;" + $durationMs)
        __NoctrailEmit "CommandEnd"
        $global:__NoctrailCommandActive = $false
    }
    if ($global:__NoctrailOriginalPrompt) {
        & $global:__NoctrailOriginalPrompt
    } else {
        "PS $((Get-Location).Path)> "
    }
}

if (Get-Module -ListAvailable -Name PSReadLine) {
    Import-Module PSReadLine
    Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
        $line = $null
        $cursor = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $global:__NoctrailCommandActive = $true
        $global:__NoctrailCommandStart = Get-Date
        __NoctrailEmit "CommandStart"
        __NoctrailEmit ("CommandText;" + $line)
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}
"#
    .to_string()
}
