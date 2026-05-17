//! PTY/process boundary for Noctrail.

use std::{
    env,
    ffi::{OsStr, OsString},
    fmt,
    io::{Read, Write},
    path::PathBuf,
};

pub use portable_pty::ExitStatus as PtyExitStatus;
use portable_pty::{Child, CommandBuilder, MasterPty, native_pty_system};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }

    fn to_portable(self) -> portable_pty::PtySize {
        portable_pty::PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyCommand {
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
    clear_env: bool,
}

impl PtyCommand {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            clear_env: false,
        }
    }

    pub fn shell() -> Self {
        Self::new(resolve_default_shell_program())
    }

    pub fn program(&self) -> &OsStr {
        &self.program
    }

    pub fn argv(&self) -> &[OsString] {
        &self.args
    }

    pub fn cwd(&self) -> Option<&PathBuf> {
        self.cwd.as_ref()
    }

    pub fn env(&self) -> &[(OsString, OsString)] {
        &self.env
    }

    pub fn clear_env(&mut self) -> &mut Self {
        self.clear_env = true;
        self
    }

    pub fn arg(&mut self, arg: impl Into<OsString>) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd_path(&mut self, cwd: impl Into<PathBuf>) -> &mut Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env_var(&mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> &mut Self {
        self.env.push((key.into(), value.into()));
        self
    }

    fn into_portable(self) -> CommandBuilder {
        let mut builder = CommandBuilder::new(self.program);

        if let Some(cwd) = self.cwd {
            builder.cwd(cwd);
        }

        if self.clear_env {
            builder.env_clear();
        }

        for (key, value) in self.env {
            builder.env(key, value);
        }

        for arg in self.args {
            builder.arg(arg);
        }

        builder
    }
}

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("{context}: {message}")]
    Context {
        context: &'static str,
        message: String,
    },
    #[error("failed to read PTY output: {0}")]
    Read(#[source] std::io::Error),
    #[error("failed to write PTY input: {0}")]
    Write(#[source] std::io::Error),
    #[error("failed to wait for child process: {0}")]
    Wait(#[source] std::io::Error),
    #[error("failed to terminate child process: {0}")]
    Kill(#[source] std::io::Error),
}

impl PtyError {
    fn context(context: &'static str, source: impl fmt::Display) -> Self {
        Self::Context {
            context,
            message: source.to_string(),
        }
    }
}

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    size: PtySize,
    closed: bool,
}

impl fmt::Debug for PtySession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtySession")
            .field("size", &self.size)
            .field("process_id", &self.process_id())
            .field("closed", &self.closed)
            .finish()
    }
}

impl PtySession {
    pub fn spawn(command: PtyCommand, size: PtySize) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size.to_portable())
            .map_err(|error| PtyError::context("failed to open PTY", error))?;
        let child = pair
            .slave
            .spawn_command(command.into_portable())
            .map_err(|error| PtyError::context("failed to spawn command in PTY", error))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| PtyError::context("failed to clone PTY reader", error))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| PtyError::context("failed to take PTY writer", error))?;

        Ok(Self {
            master: pair.master,
            child,
            reader,
            writer,
            size,
            closed: false,
        })
    }

    pub fn spawn_shell(size: PtySize) -> Result<Self, PtyError> {
        Self::spawn(PtyCommand::shell(), size)
    }

    pub fn size(&self) -> PtySize {
        self.size
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, PtyError> {
        self.reader.read(buf).map_err(PtyError::Read)
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, PtyError> {
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .map_err(PtyError::Write)?;
        Ok(bytes.len())
    }

    pub fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(size.to_portable())
            .map_err(|error| PtyError::context("failed to resize PTY", error))?;
        self.size = size;
        Ok(())
    }

    pub fn try_wait(&mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        self.child.try_wait().map_err(PtyError::Wait)
    }

    pub fn wait(&mut self) -> Result<PtyExitStatus, PtyError> {
        self.child.wait().map_err(PtyError::Wait)
    }

    pub fn kill(&mut self) -> Result<(), PtyError> {
        self.child.kill().map_err(PtyError::Kill)
    }

    pub fn close(mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        if self.closed {
            return Ok(None);
        }

        self.closed = true;

        if let Some(status) = self.child.try_wait().map_err(PtyError::Wait)? {
            return Ok(Some(status));
        }

        let _ = self.child.kill();
        self.child.wait().map(Some).map_err(PtyError::Wait)
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn resolve_default_shell_program() -> OsString {
    #[cfg(windows)]
    {
        env::var_os("COMSPEC")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("cmd.exe"))
    }

    #[cfg(not(windows))]
    {
        env::var_os("SHELL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("/bin/sh"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as StdError;

    #[test]
    fn shell_command_resolves_to_a_program() {
        let command = PtyCommand::shell();
        assert!(!command.program().is_empty());
    }

    #[test]
    fn builder_collects_args_env_and_cwd() {
        let mut command = PtyCommand::new("program");
        command
            .arg("one")
            .args(["two", "three"])
            .cwd_path("C:/tmp")
            .env_var("A", "1")
            .clear_env();

        assert_eq!(command.program(), OsStr::new("program"));
        assert_eq!(
            command.argv(),
            &[
                OsString::from("one"),
                OsString::from("two"),
                OsString::from("three")
            ]
        );
        assert_eq!(command.cwd(), Some(&PathBuf::from("C:/tmp")));
        assert_eq!(command.env(), &[(OsString::from("A"), OsString::from("1"))]);
        assert!(command.clear_env);
    }

    #[test]
    fn spawn_shell_accepts_input_and_closes() -> Result<(), Box<dyn StdError>> {
        let mut session = PtySession::spawn_shell(PtySize::new(80, 24))?;
        session.resize(PtySize::new(100, 30))?;
        assert_eq!(session.size(), PtySize::new(100, 30));

        let bytes_written = session.write(b"echo PTY_OK\r")?;
        assert_eq!(bytes_written, "echo PTY_OK\r".len());
        assert!(session.close()?.is_some());
        Ok(())
    }
}
