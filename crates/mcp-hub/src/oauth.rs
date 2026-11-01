//! The hub as an OAuth client of remote MCP servers: one sign-in per server,
//! with PKCE, discovery and refresh left to the SDK. Tokens stay with the host
//! and the hub. No agent ever receives one, since agents only talk to the hub.
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession, CredentialStore,
    StoredCredentials,
};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
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

const SIGNED_IN: &str = "<!doctype html><title>Signed in</title><p>Signed in. You can close this tab and go back to the app.</p>";

/// Waits for the browser to come back to the loopback redirect, and returns
/// the whole URL it came back to.
fn await_redirect(listener: TcpListener, deadline: Instant) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    loop {
        let (mut stream, _) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("The sign-in timed out".into());
                }
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
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
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{SIGNED_IN}",
            SIGNED_IN.len()
        );
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        return Ok(format!("http://{address}{target}"));
    }
}

/// One sign-in, with the browser opened by `open`. Blocks until the browser
/// returns or `timeout` passes, and saves the result to `store`.
pub fn sign_in(
    url: &str,
    client_name: &str,
    store: Arc<dyn TokenStore>,
    open: impl FnOnce(&str),
    timeout: Duration,
) -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let redirect = format!(
        "http://{}/callback",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let runtime = runtime()?;
    let session = runtime.block_on(async {
        let mut manager = manager(url, store).await?;
        let found = manager
            .resolve_metadata_from_challenge(None)
            .await
            .map_err(|e| e.to_string())?;
        manager.set_metadata(found.metadata);
        let request = AuthorizationRequest::new(redirect).with_client_name(client_name);
        AuthorizationSession::new(manager, request)
            .await
            .map_err(|(_, error)| error.to_string())
    })?;
    open(session.get_authorization_url());
    let returned = await_redirect(listener, Instant::now() + timeout)?;
    runtime
        .block_on(session.handle_callback_url(&returned))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A current access token, refreshed if it has expired.
pub(crate) fn access_token(url: &str, store: Arc<dyn TokenStore>) -> Result<String, String> {
    const NEEDS_SIGN_IN: &str = "This server needs a sign-in";
    let runtime = runtime()?;
    runtime.block_on(async {
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
