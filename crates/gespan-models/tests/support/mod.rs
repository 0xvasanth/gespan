//! A tiny HTTP/1.1 server for download tests. Honours `Range`, can cut a body
//! short, and records every request.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug)]
pub struct Recorded {
    pub path: String,
    pub range: Option<String>,
}

/// What to answer for one request.
pub struct Reply {
    pub body: Vec<u8>,
    /// Send only this many body bytes (of the ranged slice), then close.
    pub cut_at: Option<usize>,
    /// Answer `200` with the whole body even when a `Range` was sent.
    pub ignore_range: bool,
}

pub struct TestServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl TestServer {
    /// `handler(path, nth_request_for_that_path)` picks the reply.
    pub fn start(handler: impl Fn(&str, usize) -> Reply + Send + 'static) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &handler, &log);
            }
        });
        TestServer { base, requests }
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

/// A base URL that refuses connections.
pub fn dead_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

fn serve(
    mut stream: TcpStream,
    handler: &impl Fn(&str, usize) -> Reply,
    log: &Mutex<Vec<Recorded>>,
) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.lines();
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    let range = lines.find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case("range")
            .then(|| v.trim().to_string())
    });

    let nth = {
        let mut log = log.lock().unwrap();
        let nth = log.iter().filter(|r| r.path == path).count();
        log.push(Recorded {
            path: path.clone(),
            range: range.clone(),
        });
        nth
    };

    let reply = handler(&path, nth);
    let total = reply.body.len();
    let start = range
        .as_deref()
        .filter(|_| !reply.ignore_range)
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split('-').next())
        .and_then(|n| n.parse::<usize>().ok());
    let (status, slice) = match start {
        Some(s) => ("206 Partial Content", &reply.body[s.min(total)..]),
        None => ("200 OK", &reply.body[..]),
    };
    let mut out = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        slice.len()
    );
    if let Some(s) = start {
        out.push_str(&format!(
            "Content-Range: bytes {s}-{}/{total}\r\n",
            total - 1
        ));
    }
    out.push_str("\r\n");
    let _ = stream.write_all(out.as_bytes());
    let sent = reply.cut_at.map_or(slice.len(), |c| c.min(slice.len()));
    let _ = stream.write_all(&slice[..sent]);
    let _ = stream.flush();
}
