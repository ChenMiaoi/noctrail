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
    let file = open_log_file(&log_file)?;
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
    default_log_path_from_home(std::env::var_os("HOME").map(PathBuf::from))
}

fn default_log_path_from_home(home: Option<PathBuf>) -> PathBuf {
    if let Some(home) = home {
        return home.join(".noctrail").join("logs").join("noctrail-app.log");
    }

    std::env::temp_dir()
        .join("noctrail")
        .join("logs")
        .join("noctrail-app.log")
}

fn open_log_file(log_file: &Path) -> io::Result<File> {
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    OpenOptions::new().create(true).append(true).open(log_file)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("noctrail-logging-{label}-{unique}"))
    }

    #[test]
    fn default_log_path_uses_home_when_available() {
        let fake_home = unique_temp_dir("home");
        let path = default_log_path_from_home(Some(fake_home.clone()));

        assert_eq!(
            path,
            fake_home
                .join(".noctrail")
                .join("logs")
                .join("noctrail-app.log")
        );
    }

    #[test]
    fn open_log_file_creates_missing_parent_directories() {
        let temp_dir = unique_temp_dir("parents");
        let log_file = temp_dir.join("nested").join("logs").join("app.log");

        let file = open_log_file(&log_file).expect("log file should be created");
        drop(file);

        assert!(log_file.exists());
        assert!(
            log_file
                .parent()
                .expect("log file should have a parent")
                .is_dir()
        );

        let _ = fs::remove_file(&log_file);
        let _ = fs::remove_dir_all(temp_dir);
    }
}
