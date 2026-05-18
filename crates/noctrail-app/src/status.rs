use std::{
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use noctrail_pty::{PtyCommand, PtyExitStatus};
use noctrail_term::LineEnding;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaneStatusLine {
    pub shell: Option<String>,
    pub cwd: Option<PathBuf>,
    pub git_branch: Option<String>,
    pub exit_status: Option<String>,
}

impl PaneStatusLine {
    pub(crate) fn from_command(command: &PtyCommand) -> Self {
        let cwd = command.cwd().cloned();

        Self {
            shell: Some(command_shell_label(command)),
            cwd: cwd.clone(),
            git_branch: cwd.as_deref().and_then(detect_git_branch),
            exit_status: None,
        }
    }
}

pub(crate) fn selection_line_ending() -> LineEnding {
    if cfg!(windows) {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

pub(crate) fn format_exit_status(status: &PtyExitStatus) -> String {
    match status.signal() {
        Some(signal) => format!("signal {signal}"),
        None => format!("code {}", status.exit_code()),
    }
}

fn command_shell_label(command: &PtyCommand) -> String {
    Path::new(command.program())
        .file_name()
        .unwrap_or(command.program())
        .to_string_lossy()
        .into_owned()
}

fn detect_git_branch(cwd: &Path) -> Option<String> {
    for args in [
        ["symbolic-ref", "--quiet", "--short", "HEAD"].as_slice(),
        ["rev-parse", "--abbrev-ref", "HEAD"].as_slice(),
    ] {
        let output = ProcessCommand::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .ok()?;
        if !output.status.success() {
            continue;
        }

        let branch = String::from_utf8(output.stdout).ok()?;
        let branch = branch.trim();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch.to_string());
        }
    }

    None
}
