use std::{
    io::Write,
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
};

pub(crate) fn configure_pty_writer(writer: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
    Box::new(AsyncPtyWriter::new(writer))
}

#[derive(Debug)]
struct AsyncPtyWriter {
    tx: Option<mpsc::Sender<WriterMessage>>,
    handle: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

#[derive(Debug)]
enum WriterMessage {
    Bytes(Vec<u8>),
    Flush,
    Close,
}

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
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY writer closed")
            })?
            .send(message)
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY writer thread stopped")
            })
    }
}

impl Write for AsyncPtyWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.send(WriterMessage::Bytes(buf.to_vec()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send(WriterMessage::Flush)
    }
}

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
