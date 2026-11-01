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
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
};

/// Messages waiting on a connection. Far more than any agent has in flight, so
/// reaching it means the connection has stalled.
const MAX_QUEUED_MESSAGES: usize = 4096;

/// What the hub writes to a connection. A message is written in pieces and goes
/// on the queue whole at `flush`, so a full queue drops it whole rather than
/// leaving half a line for the next one to run into. It queues rather than
/// blocks, so a stalled connection can't hold up the stop that would end it.
struct Queue {
    sender: mpsc::SyncSender<Vec<u8>>,
    message: Vec<u8>,
}

impl Queue {
    fn new(sender: mpsc::SyncSender<Vec<u8>>) -> Self {
        Self {
            sender,
            message: Vec::new(),
        }
    }
}

impl Write for Queue {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.message.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.message.is_empty() {
            return Ok(());
        }
        self.sender
            .try_send(std::mem::take(&mut self.message))
            .map_err(|_| std::io::ErrorKind::BrokenPipe.into())
    }
}

/// The other end of a [`Queue`], which ends when that's closed.
struct Queued {
    frames: mpsc::Receiver<Vec<u8>>,
    current: std::io::Cursor<Vec<u8>>,
    /// Set by a stop: what's still waiting is for nobody now.
    stopped: Arc<AtomicBool>,
}

impl Read for Queued {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                return Ok(0);
            }
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
    let (to_remote, frames) = mpsc::sync_channel(MAX_QUEUED_MESSAGES);
    let stopped = Arc::new(AtomicBool::new(false));
    let to_read = Queued {
        frames,
        current: std::io::Cursor::default(),
        stopped: stopped.clone(),
    };
    let (from_remote, to_host) = std::io::pipe()?;
    let input = Closable::new(Queue::new(to_remote));
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
        stop: Box::new(move || {
            stopped.store(true, Ordering::SeqCst);
            stop_input.close();
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> (Queue, Queued, Arc<AtomicBool>) {
        let (sender, frames) = mpsc::sync_channel(2);
        let stopped = Arc::new(AtomicBool::new(false));
        let queued = Queued {
            frames,
            current: std::io::Cursor::default(),
            stopped: stopped.clone(),
        };
        (Queue::new(sender), queued, stopped)
    }

    fn send(queue: &mut Queue, message: &str) -> std::io::Result<()> {
        // In pieces, the way `writeln!` writes a frame.
        for piece in message.as_bytes().chunks(2) {
            queue.write_all(piece)?;
        }
        queue.write_all(b"\n")?;
        queue.flush()
    }

    #[test]
    fn a_full_queue_drops_a_message_whole_and_counts_messages_not_pieces() {
        let (mut queue, queued, _) = queue();
        send(&mut queue, "first message").unwrap();
        send(&mut queue, "second").unwrap();
        assert!(send(&mut queue, "dropped").is_err());
        drop(queue);
        let mut lines = String::new();
        BufReader::new(queued).read_to_string(&mut lines).unwrap();
        assert_eq!(lines, "first message\nsecond\n");
    }

    #[test]
    fn a_message_after_a_dropped_one_starts_on_its_own_line() {
        let (mut queue, mut queued, _) = queue();
        send(&mut queue, "one").unwrap();
        send(&mut queue, "two").unwrap();
        assert!(send(&mut queue, "dropped").is_err());
        let mut first = [0; 4];
        queued.read_exact(&mut first).unwrap();
        send(&mut queue, "three").unwrap();
        drop(queue);
        let mut rest = String::new();
        queued.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "two\nthree\n");
    }

    #[test]
    fn a_stop_drops_what_was_still_waiting() {
        let (mut queue, mut queued, stopped) = queue();
        send(&mut queue, "one").unwrap();
        stopped.store(true, Ordering::SeqCst);
        let mut read = Vec::new();
        queued.read_to_end(&mut read).unwrap();
        assert!(read.is_empty());
    }
}
