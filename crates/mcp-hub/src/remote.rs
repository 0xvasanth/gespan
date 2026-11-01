//! A remote server behind the same stdio shape as a local one, so everything
//! the pool does applies to it unchanged. Frames written to it go out over
//! HTTP, and what comes back is read as if from a process's stdout.
use crate::{
    oauth::{access_token, TokenStore},
    relay::{pump, Credential, Link, TokenSource},
    Spawned,
};
use std::{
    collections::BTreeMap,
    io::BufReader,
    sync::{Arc, Mutex},
};

/// Connects to a remote server. With `oauth`, each request carries a current
/// token from the store; `headers` are sent as they are.
pub fn connect(
    url: &str,
    headers: BTreeMap<String, String>,
    oauth: Option<Arc<dyn TokenStore>>,
) -> std::io::Result<Spawned> {
    let (to_read, to_remote) = std::io::pipe()?;
    let (from_remote, to_host) = std::io::pipe()?;
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
    let output = Arc::new(Mutex::new(to_host));
    std::thread::Builder::new()
        .name("mcp-remote".into())
        .spawn(move || {
            if let Err(error) = pump(link, BufReader::new(to_read), output) {
                tracing::warn!(%error, "a remote MCP connection ended");
            }
        })?;
    Ok(Spawned {
        pid: 0,
        stdin: Box::new(to_remote),
        stdout: Box::new(from_remote),
        stop: Box::new(|| {}),
    })
}
