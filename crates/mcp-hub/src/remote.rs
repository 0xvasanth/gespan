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
    io::{BufReader, Write},
    sync::{Arc, Mutex},
};

/// A pipe end anyone holding it can close. Closing is how a connection stops:
/// the thread on the other end sees the pipe end and winds up.
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
    let (to_read, to_remote) = std::io::pipe()?;
    let (from_remote, to_host) = std::io::pipe()?;
    let (input, stop_input) = {
        let input = Closable::new(to_remote);
        (input.clone(), input)
    };
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
            // The listening thread holds this too, for as long as its stream
            // stays open, so the reader can't wait for that.
            to_host.close();
        })?;
    Ok(Spawned {
        pid: 0,
        stdin: Box::new(input),
        stdout: Box::new(from_remote),
        stop: Box::new(move || stop_input.close()),
    })
}
