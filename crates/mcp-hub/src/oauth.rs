//! The hub as an OAuth client of remote MCP servers: one sign-in per server,
//! with PKCE, discovery and refresh left to the SDK. Tokens stay with the host
//! and the hub. No agent ever receives one, since agents only talk to the hub.
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationMetadata, AuthorizationRequest,
    AuthorizationSession, CredentialStore, StoredCredentials,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};

/// Where a server's credentials live. Hosts should use the OS keychain.
pub trait TokenStore: Send + Sync + 'static {
    fn load(&self) -> Result<Option<String>, String>;
    fn save(&self, value: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

struct Stored(Arc<dyn TokenStore>);

fn store_failed(error: impl std::fmt::Display) -> AuthError {
    AuthError::InternalError(error.to_string())
}

#[async_trait::async_trait]
impl CredentialStore for Stored {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match self.0.load().map_err(store_failed)? {
            Some(text) => serde_json::from_str(&text).map(Some).map_err(store_failed),
            None => Ok(None),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials).map_err(store_failed)?;
        self.0.save(&text).map_err(store_failed)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.0.clear().map_err(store_failed)
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    // Fails harmlessly once one is installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

async fn manager(url: &str, store: Arc<dyn TokenStore>) -> Result<AuthorizationManager, String> {
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|e| e.to_string())?;
    manager.set_credential_store(Stored(store));
    Ok(manager)
}

/// A token sent to this address can't be read on the way.
pub fn safe_for_tokens(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| match url.scheme() {
        "https" => true,
        "http" => matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
        _ => false,
    })
}

/// A server names where sign-in goes, so that has to be as safe as the server.
fn safe_endpoints(metadata: &AuthorizationMetadata) -> bool {
    [
        Some(&metadata.authorization_endpoint),
        Some(&metadata.token_endpoint),
        metadata.registration_endpoint.as_ref(),
    ]
    .into_iter()
    .flatten()
    .all(|endpoint| safe_for_tokens(endpoint))
}

const TIMED_OUT: &str = "The sign-in timed out";
/// How long one stray connection to the redirect port can hold it up.
const REQUEST_WAIT: Duration = Duration::from_secs(5);
const MAX_REQUEST_LINE: usize = 8 * 1024;
const UNSAFE: &str = "This server signs in over an unsafe address";

const SIGNED_IN: &str = "<!doctype html><title>Signed in</title><p>Signed in. You can close this tab and go back to the app.</p>";

/// The first line of a request, or nothing by `deadline`. Each read waits only
/// for what's left, so a client sending a byte at a time can't stretch it.
fn request_line(stream: &mut TcpStream, deadline: Instant) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0];
    while line.len() < MAX_REQUEST_LINE {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        match stream.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => return String::from_utf8(line).ok(),
            Ok(1) => line.push(byte[0]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            _ => return None,
        }
    }
    None
}

/// Waits for the browser to come back to the loopback redirect, and returns
/// the whole URL it came back to.
fn await_redirect(listener: TcpListener, deadline: Instant) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    loop {
        let (mut stream, _) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(TIMED_OUT.into());
                }
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(TIMED_OUT.into());
        }
        stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        let cutoff = Instant::now() + left.min(REQUEST_WAIT);
        let Some(line) = request_line(&mut stream, cutoff) else {
            continue;
        };
        // Anything else that finds this port, a favicon say, is ignored.
        let Some(target) = line
            .strip_prefix("GET ")
            .and_then(|rest| rest.split(' ').next())
            .filter(|target| target.starts_with("/callback?"))
        else {
            let _ = write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
            );
            continue;
        };
        // Closing with the headers unread can reset the connection, and the
        // browser would lose the page.
        while request_line(&mut stream, cutoff).is_some_and(|line| !line.trim().is_empty()) {}
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{SIGNED_IN}",
            SIGNED_IN.len()
        );
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        return Ok(format!("http://{address}{target}"));
    }
}

/// One sign-in, with the browser opened by `open`. Blocks until it's done or
/// `timeout` passes, and saves the result to `store`.
pub fn sign_in(
    url: &str,
    client_name: &str,
    store: Arc<dyn TokenStore>,
    open: impl FnOnce(&str),
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let redirect = format!(
        "http://{}/callback",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let runtime = runtime()?;
    let session = runtime
        .block_on(async {
            tokio::time::timeout(timeout, async {
                let mut manager = manager(url, store).await?;
                let found = manager
                    .resolve_metadata_from_challenge(None)
                    .await
                    .map_err(|e| e.to_string())?;
                if !safe_endpoints(&found.metadata) {
                    return Err(UNSAFE.to_owned());
                }
                manager.set_metadata(found.metadata);
                let request = AuthorizationRequest::new(redirect).with_client_name(client_name);
                AuthorizationSession::new(manager, request)
                    .await
                    .map_err(|(_, error)| error.to_string())
            })
            .await
        })
        .map_err(|_| TIMED_OUT.to_owned())??;
    open(session.get_authorization_url());
    let returned = await_redirect(listener, deadline)?;
    let left = deadline.saturating_duration_since(Instant::now());
    runtime
        .block_on(async {
            tokio::time::timeout(left, session.handle_callback_url(&returned)).await
        })
        .map_err(|_| TIMED_OUT.to_owned())?
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A current access token, refreshed if it has expired.
pub(crate) fn access_token(url: &str, store: Arc<dyn TokenStore>) -> Result<String, String> {
    const NEEDS_SIGN_IN: &str = "This server needs a sign-in";
    let runtime = runtime()?;
    runtime.block_on(async {
        // A refresh sends the refresh token wherever the server says today,
        // and the SDK keeps that to itself, so look first.
        let found = manager(url, store.clone())
            .await?
            .resolve_metadata_from_challenge(None)
            .await
            .map_err(|e| e.to_string())?;
        if !safe_endpoints(&found.metadata) {
            return Err(UNSAFE.to_owned());
        }
        let mut manager = manager(url, store).await?;
        if !manager
            .initialize_from_store()
            .await
            .map_err(|e| e.to_string())?
        {
            return Err(NEEDS_SIGN_IN.to_owned());
        }
        manager
            .get_access_token()
            .await
            .map_err(|_| NEEDS_SIGN_IN.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[test]
    fn tokens_only_travel_over_https_or_stay_on_this_machine() {
        for url in [
            "https://mcp.example/mcp",
            "HTTPS://mcp.example/mcp",
            "http://127.0.0.1:9/mcp",
            "http://localhost:9/mcp",
            "http://[::1]:9/mcp",
        ] {
            assert!(safe_for_tokens(url), "{url}");
        }
        for url in [
            "http://mcp.example/mcp",
            "http://127.0.0.1.mcp.example/mcp",
            "http://localhost.evil.example/mcp",
            "ws://127.0.0.1:9/mcp",
            "not a url",
        ] {
            assert!(!safe_for_tokens(url), "{url}");
        }
    }

    #[test]
    fn a_client_sending_a_byte_at_a_time_cant_outlast_the_sign_in() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            while stream.write_all(b"G").is_ok() {
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let started = Instant::now();
        let deadline = started + Duration::from_millis(200);
        assert_eq!(await_redirect(listener, deadline).unwrap_err(), TIMED_OUT);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_client_that_connects_and_says_nothing_cant_outlast_the_sign_in() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _silent = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let started = Instant::now();
        let deadline = started + Duration::from_millis(150);
        assert_eq!(await_redirect(listener, deadline).unwrap_err(), TIMED_OUT);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
