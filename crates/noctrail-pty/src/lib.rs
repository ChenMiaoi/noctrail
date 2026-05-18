//! PTY/process boundary for Noctrail.

use std::{
    env,
    ffi::{OsStr, OsString},
    fmt,
    io::{Read, Write},
    path::PathBuf,
};

#[cfg(windows)]
use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellSource {
    EnvShell,
    EnvComSpec,
    PathPwsh,
    PathPowerShell,
    PathWsl,
    FallbackSh,
    FallbackCmd,
}

impl ShellSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::EnvShell => "env:SHELL",
            Self::EnvComSpec => "env:COMSPEC",
            Self::PathPwsh => "path:pwsh",
            Self::PathPowerShell => "path:powershell",
            Self::PathWsl => "path:wsl",
            Self::FallbackSh => "fallback:/bin/sh",
            Self::FallbackCmd => "fallback:cmd.exe",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShell {
    command: PtyCommand,
    source: ShellSource,
    cwd: Option<PathBuf>,
}

impl ResolvedShell {
    pub fn detect() -> Self {
        let cwd = env::current_dir().ok();
        let (program, source) = detect_shell_program(env::vars_os(), env::split_paths);
        let mut command = PtyCommand::new(program.clone());
        #[cfg(windows)]
        configure_windows_shell_command(&mut command, &program, source);
        if let Some(path) = cwd.clone() {
            command.cwd_path(path);
        }

        Self {
            command,
            source,
            cwd,
        }
    }

    pub fn command(&self) -> &PtyCommand {
        &self.command
    }

    pub fn source(&self) -> ShellSource {
        self.source
    }

    pub fn cwd(&self) -> Option<&PathBuf> {
        self.cwd.as_ref()
    }

    pub fn inherits_env(&self) -> bool {
        self.command.inherits_env()
    }

    pub fn env_overrides(&self) -> &[(OsString, OsString)] {
        self.command.env()
    }

    pub fn into_command(self) -> PtyCommand {
        self.command
    }
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
        ResolvedShell::detect().into_command()
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

    pub fn inherits_env(&self) -> bool {
        !self.clear_env
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
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn Child + Send>>,
    reader: Option<Box<dyn Read + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    size: PtySize,
    closed: bool,
}

pub struct PtyOutputReader {
    inner: Box<dyn Read + Send>,
}

impl fmt::Debug for PtyOutputReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PtyOutputReader").finish_non_exhaustive()
    }
}

impl Read for PtyOutputReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
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
        let writer = configure_pty_writer(writer);

        Ok(Self {
            master: Some(pair.master),
            child: Some(child),
            reader: Some(reader),
            writer: Some(writer),
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
        self.child.as_ref().and_then(|child| child.process_id())
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, PtyError> {
        match self.reader.as_mut() {
            Some(reader) => reader.read(buf).map_err(PtyError::Read),
            None => Err(closed_error("read from")),
        }
    }

    pub fn clone_output_reader(&self) -> Result<PtyOutputReader, PtyError> {
        let master = self
            .master
            .as_ref()
            .ok_or_else(|| closed_error("clone reader from"))?;
        let reader = master
            .try_clone_reader()
            .map_err(|error| PtyError::context("failed to clone PTY reader", error))?;
        Ok(PtyOutputReader { inner: reader })
    }

    pub fn take_output_reader(&mut self) -> Result<PtyOutputReader, PtyError> {
        let reader = self
            .reader
            .take()
            .ok_or_else(|| closed_error("take reader from"))?;
        Ok(PtyOutputReader { inner: reader })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, PtyError> {
        let writer = match self.writer.as_mut() {
            Some(writer) => writer,
            None => return Err(closed_error("write to")),
        };
        writer.write_all(bytes).map_err(PtyError::Write)?;
        Ok(bytes.len())
    }

    pub fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        match self.master.as_ref() {
            Some(master) => master
                .resize(size.to_portable())
                .map_err(|error| PtyError::context("failed to resize PTY", error))?,
            None => return Err(closed_error("resize")),
        }
        self.size = size;
        Ok(())
    }

    pub fn try_wait(&mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        match self.child.as_mut() {
            Some(child) => child.try_wait().map_err(PtyError::Wait),
            None => Err(closed_error("wait on")),
        }
    }

    pub fn wait(&mut self) -> Result<PtyExitStatus, PtyError> {
        match self.child.as_mut() {
            Some(child) => child.wait().map_err(PtyError::Wait),
            None => Err(closed_error("wait on")),
        }
    }

    pub fn kill(&mut self) -> Result<(), PtyError> {
        match self.child.as_mut() {
            Some(child) => child.kill().map_err(PtyError::Kill),
            None => Err(closed_error("wait on")),
        }
    }

    pub fn close(mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        if self.closed {
            return Ok(None);
        }

        self.closed = true;

        if let Some(status) = self.try_wait()? {
            return Ok(Some(status));
        }

        // Drop master-side PTY handles before forcing shutdown so shells
        // that are already unwinding can observe hangup/EOF promptly.
        self.writer.take();
        self.reader.take();
        self.master.take();

        if let Some(status) = self.try_wait()? {
            return Ok(Some(status));
        }

        let _ = self.kill();
        self.wait().map(Some)
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn detect_shell_program<I, F>(env_vars: I, _split_paths: F) -> (OsString, ShellSource)
where
    I: IntoIterator<Item = (OsString, OsString)>,
    F: Fn(&OsStr) -> env::SplitPaths,
{
    #[cfg(windows)]
    {
        detect_windows_shell_program(env_vars, _split_paths)
    }

    #[cfg(not(windows))]
    {
        detect_unix_shell_program(env_vars)
    }
}

#[cfg(not(windows))]
fn detect_unix_shell_program<I>(env_vars: I) -> (OsString, ShellSource)
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

#[cfg(windows)]
fn detect_windows_shell_program<I, F>(env_vars: I, split_paths: F) -> (OsString, ShellSource)
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
        let candidate = PathBuf::from(root).join("PowerShell").join("7").join("pwsh.exe");
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

#[cfg(windows)]
fn configure_pty_writer(writer: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
    Box::new(AsyncPtyWriter::new(writer))
}

#[cfg(not(windows))]
fn configure_pty_writer(writer: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
    writer
}

#[cfg(windows)]
#[derive(Debug)]
struct AsyncPtyWriter {
    tx: Option<mpsc::Sender<WriterMessage>>,
    handle: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

#[cfg(windows)]
#[derive(Debug)]
enum WriterMessage {
    Bytes(Vec<u8>),
    Flush,
    Close,
}

#[cfg(windows)]
impl AsyncPtyWriter {
    fn new(mut sink: Box<dyn Write + Send>) -> Self {
        let (tx, rx) = mpsc::channel::<WriterMessage>();
        let error = Arc::new(Mutex::new(None));
        let writer_error = Arc::clone(&error);
        let handle = thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                let result = match message {
                    WriterMessage::Bytes(bytes) => sink.write_all(&bytes),
                    WriterMessage::Flush => sink.flush(),
                    WriterMessage::Close => {
                        let _ = sink.flush();
                        break;
                    }
                };
                if let Err(err) = result {
                    *writer_error
                        .lock()
                        .expect("async PTY writer error lock should not be poisoned") =
                        Some(err.to_string());
                    break;
                }
            }
        });

        Self {
            tx: Some(tx),
            handle: Some(handle),
            error,
        }
    }

    fn take_error(&self) -> Option<String> {
        self.error
            .lock()
            .expect("async PTY writer error lock should not be poisoned")
            .clone()
    }

    fn send(&self, message: WriterMessage) -> std::io::Result<()> {
        if let Some(error) = self.take_error() {
            return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, error));
        }
        self.tx
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY writer closed"))?
            .send(message)
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY writer thread stopped")
            })
    }
}

#[cfg(windows)]
impl Write for AsyncPtyWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.send(WriterMessage::Bytes(buf.to_vec()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send(WriterMessage::Flush)
    }
}

#[cfg(windows)]
impl Drop for AsyncPtyWriter {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(WriterMessage::Close);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(windows)]
fn program_exists_on_path<F>(path_value: Option<&OsStr>, program: &str, split_paths: &F) -> bool
where
    F: Fn(&OsStr) -> env::SplitPaths,
{
    let Some(path_value) = path_value else {
        return false;
    };

    split_paths(path_value).any(|dir| {
        let candidate = dir.join(program);
        path_is_executable(&candidate)
    })
}

#[cfg(windows)]
fn path_is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(windows)]
fn configure_windows_shell_command(command: &mut PtyCommand, program: &OsStr, source: ShellSource) {
    if !matches!(source, ShellSource::PathPwsh | ShellSource::PathPowerShell) {
        return;
    }

    let shell = Path::new(program)
        .file_name()
        .unwrap_or(program)
        .to_string_lossy()
        .to_ascii_lowercase();
    if !matches!(shell.as_str(), "pwsh.exe" | "pwsh" | "powershell.exe" | "powershell") {
        return;
    }

    command
        .arg("-NoLogo")
        .arg("-NoExit")
        .arg("-Command")
        .arg(render_powershell_hook());
}

#[cfg(windows)]
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

fn closed_error(action: &'static str) -> PtyError {
    PtyError::context(
        "PTY session already closed",
        format!("cannot {action} a closed PTY session"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as StdError;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn shell_command_resolves_to_a_program() {
        let command = PtyCommand::shell();
        assert!(!command.program().is_empty());
    }

    #[test]
    fn resolved_shell_reports_cwd_and_env_mode() {
        let shell = ResolvedShell::detect();
        assert!(!shell.command().program().is_empty());
        assert!(shell.inherits_env());
        assert!(shell.cwd().is_some());
        assert!(shell.env_overrides().is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_shell_prefers_shell_env() {
        let env_vars = vec![(OsString::from("SHELL"), OsString::from("/bin/fish"))];
        let (program, source) = detect_unix_shell_program(env_vars);
        assert_eq!(program, OsString::from("/bin/fish"));
        assert_eq!(source, ShellSource::EnvShell);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_shell_falls_back_to_bin_sh() {
        let (program, source) = detect_unix_shell_program(Vec::<(OsString, OsString)>::new());
        assert_eq!(program, OsString::from("/bin/sh"));
        assert_eq!(source, ShellSource::FallbackSh);
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

    #[test]
    fn finite_command_reaches_eof_and_exit_status() -> Result<(), Box<dyn StdError>> {
        let mut session = PtySession::spawn(finite_command("PTY_EOF_OK"), PtySize::new(80, 24))?;

        let output = read_all_output(&mut session)?;
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("PTY_EOF_OK"),
            "finite PTY command output missing marker: {text:?}"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if session.try_wait()?.is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "finite PTY command should exit after EOF"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            session.close()?.is_some(),
            "close should reap the already-exited child handle"
        );
        Ok(())
    }

    fn read_all_output(session: &mut PtySession) -> Result<Vec<u8>, PtyError> {
        let mut output = Vec::new();
        let mut chunk = [0_u8; 1024];

        loop {
            let count = session.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            output.extend_from_slice(&chunk[..count]);
        }

        Ok(output)
    }

    fn finite_command(marker: &str) -> PtyCommand {
        #[cfg(windows)]
        {
            let mut command = PtyCommand::new("cmd.exe");
            command.args(["/C", "echo", marker]);
            command
        }

        #[cfg(not(windows))]
        {
            let mut command = PtyCommand::new("sh");
            command.args(["-lc", &format!("printf '{marker}\\n'")]);
            command
        }
    }
}
