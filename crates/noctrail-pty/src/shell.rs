use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use super::{PtyCommand, ShellSource};

#[cfg(windows)]
mod platform;

pub(crate) fn detect_shell_program<I, F>(env_vars: I, split_paths: F) -> (OsString, ShellSource)
where
    I: IntoIterator<Item = (OsString, OsString)>,
    F: Fn(&OsStr) -> env::SplitPaths,
{
    #[cfg(windows)]
    {
        platform::detect_shell_program(env_vars, split_paths)
    }

    #[cfg(not(windows))]
    {
        let _ = split_paths;
        detect_unix_shell_program(env_vars)
    }
}

pub(crate) fn configure_shell_command(
    command: &mut PtyCommand,
    program: &OsStr,
    source: ShellSource,
) {
    #[cfg(windows)]
    platform::configure_shell_command(command, program, source);

    #[cfg(not(windows))]
    {
        let _ = (command, program, source);
    }
}

#[cfg(not(windows))]
pub(crate) fn detect_unix_shell_program<I>(env_vars: I) -> (OsString, ShellSource)
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    for (key, value) in env_vars {
        if key == OsStr::new("SHELL") && !value.is_empty() {
            return (value, ShellSource::EnvShell);
        }
    }

    (OsString::from("/bin/sh"), ShellSource::FallbackSh)
}

#[cfg(all(not(windows), test))]
pub(crate) fn default_hooked_shell_command(marker: &str) -> PtyCommand {
    let mut command = PtyCommand::new("sh");
    command.args(["-lc", &format!("printf '{marker}\\n'")]);
    command
}

#[cfg(all(windows, test))]
pub(crate) fn default_hooked_shell_command(marker: &str) -> PtyCommand {
    let mut command = PtyCommand::new("cmd.exe");
    command.args(["/C", "echo", marker]);
    command
}

pub(crate) fn current_dir() -> Option<PathBuf> {
    env::current_dir().ok()
}
