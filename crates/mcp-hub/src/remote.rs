//! A remote server behind the same stdio shape as a local one, so everything
//! the pool does applies to it unchanged. Frames written to it go out over
//! HTTP, and what comes back is read as if from a process's stdout.
use crate::{
    oauth::{access_token, safe_for_tokens, TokenStore},
    relay::{pump, Credential, Link, TokenSource},
    Spawned,
};
use std::{
    collections::BTreeMap,
    io::{BufReader, Read, Write},
    sync::{mpsc, Arc, Mutex},
};

/// What the hub writes to a connection. It queues rather than blocks, so a
/// stalled connection can't hold up the stop that would end it.
struct Queue(mpsc::Sender<Vec<u8>>);

impl Write for Queue {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(bytes.to_vec())
            .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The other end of a [`Queue`], which ends when that's closed.
struct Queued {
    frames: mpsc::Receiver<Vec<u8>>,
    current: std::io::Cursor<Vec<u8>>,
}

impl Read for Queued {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let read = self.current.read(buffer)?;
            if read > 0 || buffer.is_empty() {
                return Ok(read);
            }
            match self.frames.recv() {
                Ok(frame) => self.current = std::io::Cursor::new(frame),
                Err(_) => return Ok(0),
            }
        }
    }
}

/// A writer anyone holding it can close. Closing is how a connection stops: the
/// thread on the other end sees the end and winds up.
struct Closable<W>(Arc<Mutex<Option<W>>>);

impl<W> Clone for Closable<W> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<W> Closable<W> {
    fn new(writer: W) -> Self {
        Self(Arc::new(Mutex::new(Some(writer))))
    }

    fn close(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl<W: Write> Write for Closable<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self.0.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            Some(writer) => writer.write(bytes),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.0.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            Some(writer) => writer.flush(),
            None => Ok(()),
        }
    }
}

/// Connects to a remote server. With `oauth`, each request carries a current
/// token from the store; `headers` are sent as they are. Neither goes to an
/// address where they could be read on the way.
pub fn connect(
    url: &str,
    headers: BTreeMap<String, String>,
    oauth: Option<Arc<dyn TokenStore>>,
) -> std::io::Result<Spawned> {
    if (oauth.is_some() || !headers.is_empty()) && !safe_for_tokens(url) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Credentials need an https address",
        ));
    }
    let (to_remote, frames) = mpsc::channel();
    let to_read = Queued {
        frames,
        current: std::io::Cursor::default(),
    };
    let (from_remote, to_host) = std::io::pipe()?;
    let input = Closable::new(Queue(to_remote));
    let stop_input = input.clone();
    let to_host = Closable::new(to_host);
    let credential = match oauth {
        Some(store) => {
            let server = url.to_owned();
            Credential::Source(Arc::new(TokenSource::new(Box::new(move || {
                access_token(&server, store.clone())
            }))))
        }
        None => Credential::None,
    };
    let link = Link::new(url.to_owned(), credential, headers);
    let output = Arc::new(Mutex::new(to_host.clone()));
    std::thread::Builder::new()
        .name("mcp-remote".into())
        .spawn(move || {
            if let Err(error) = pump(link, BufReader::new(to_read), output) {
                tracing::warn!(%error, "a remote MCP connection ended");
            }
            // The listening thread holds this too, until its next read times
            // out, and the reader can't wait for that.
            to_host.close();
        })?;
    Ok(Spawned {
        pid: 0,
        stdin: Box::new(input),
        stdout: Box::new(from_remote),
        stop: Box::new(move || stop_input.close()),
    })
}
