use std::{
    fs::{self, File, OpenOptions},
    io::{self, Stderr, Write, stderr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Debug, Clone)]
pub struct LoggingOptions {
    pub debug: bool,
    pub log_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LoggingHandle {
    log_file: PathBuf,
}

impl LoggingHandle {
    pub fn log_file(&self) -> &Path {
        &self.log_file
    }
}

pub fn init(options: &LoggingOptions) -> io::Result<LoggingHandle> {
    let log_file = options.log_file.clone().unwrap_or_else(default_log_path);
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;
    let level = if options.debug {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    };
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(options.debug)
        .with_line_number(options.debug)
        .with_ansi(false)
        .with_writer(TeeMakeWriter::new(file))
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|error| io::Error::other(format!("failed to install global logger: {error}")))?;

    Ok(LoggingHandle { log_file })
}

fn default_log_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        return home.join(".noctrail").join("logs").join("noctrail-app.log");
    }

    std::env::temp_dir()
        .join("noctrail")
        .join("logs")
        .join("noctrail-app.log")
}

#[derive(Clone)]
struct TeeMakeWriter {
    file: Arc<Mutex<File>>,
}

impl TeeMakeWriter {
    fn new(file: File) -> Self {
        Self {
            file: Arc::new(Mutex::new(file)),
        }
    }
}

impl<'a> MakeWriter<'a> for TeeMakeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            stderr: stderr(),
            file: self.file.clone(),
        }
    }
}

struct TeeWriter {
    stderr: Stderr,
    file: Arc<Mutex<File>>,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stderr.write_all(buf)?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("log file lock poisoned"))?;
        file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stderr.flush()?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("log file lock poisoned"))?;
        file.flush()
    }
}
