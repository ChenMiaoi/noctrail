use std::io::Write;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub(crate) fn configure_pty_writer(writer: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
    windows::configure_pty_writer(writer)
}

#[cfg(not(windows))]
pub(crate) fn configure_pty_writer(writer: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
    writer
}
