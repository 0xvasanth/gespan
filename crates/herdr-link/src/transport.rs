use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;

/// One open connection to a herdr server.
pub struct Conn {
    pub writer: Box<dyn Write + Send>,
    pub reader: Box<dyn BufRead + Send>,
    /// Keeps whatever carries the connection alive (an ssh child process);
    /// dropping it ends the connection.
    pub guard: Option<Box<dyn Send>>,
    /// Ends the connection from another thread, so a blocked read returns.
    pub closer: Option<Closer>,
}

/// A cloneable handle that cuts one connection. Closing twice, or after the
/// connection ended, does nothing.
#[derive(Clone)]
pub struct Closer(Arc<dyn Fn() + Send + Sync>);

impl Closer {
    pub fn new(close: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(close))
    }

    pub fn close(&self) {
        (self.0)()
    }
}

/// A way to reach a herdr server. Every `open` makes a fresh connection.
pub trait Transport: Send + Sync {
    fn open(&self) -> io::Result<Conn>;
}

/// herdr's unix socket on this machine.
#[cfg(unix)]
pub struct LocalSocket(pub PathBuf);

#[cfg(unix)]
impl Transport for LocalSocket {
    fn open(&self) -> io::Result<Conn> {
        let stream = std::os::unix::net::UnixStream::connect(&self.0)?;
        let writer = stream.try_clone()?;
        let cut = stream.try_clone()?;
        Ok(Conn {
            writer: Box::new(writer),
            reader: Box::new(io::BufReader::new(stream)),
            guard: None,
            closer: Some(Closer::new(move || {
                let _ = cut.shutdown(std::net::Shutdown::Both);
            })),
        })
    }
}

#[cfg(test)]
pub(crate) use fake::{FakeTransport, Step};

#[cfg(test)]
mod fake {
    use super::*;
    use crate::SshFailure;
    use std::collections::VecDeque;
    use std::io::{Cursor, Read};
    use std::sync::{Condvar, Mutex};

    enum Script {
        Reply(Vec<u8>),
        /// Replies, then stays open until closed.
        Endless(Vec<u8>),
        Fail(SshFailure),
    }

    #[derive(Default)]
    struct Cut {
        closed: Mutex<bool>,
        wake: Condvar,
    }

    /// Hands out its script; at the end it either reports EOF or, for an
    /// endless script, blocks until closed. Once closed it reports EOF.
    struct Reader {
        data: Cursor<Vec<u8>>,
        endless: bool,
        cut: Arc<Cut>,
    }

    impl Read for Reader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = self.fill_buf()?.len().min(out.len());
            out[..n].copy_from_slice(&self.fill_buf()?[..n]);
            self.consume(n);
            Ok(n)
        }
    }

    impl BufRead for Reader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            let mut closed = self.cut.closed.lock().unwrap();
            if !*closed && self.endless && self.data.fill_buf()?.is_empty() {
                closed = self.cut.wake.wait_while(closed, |c| !*c).unwrap();
            }
            if *closed {
                return Ok(&[]);
            }
            drop(closed);
            self.data.fill_buf()
        }
        fn consume(&mut self, amt: usize) {
            self.data.consume(amt)
        }
    }

    /// What happened to the connections, in order. Connections are numbered
    /// by successful open, so `Opened(i)` carries `requests()[i]`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Step {
        Opened(usize),
        Closed(usize),
    }

    #[derive(Default)]
    struct State {
        scripts: VecDeque<Script>,
        opens: usize,
        conns: usize,
        written: Vec<u8>,
        log: Vec<Step>,
    }

    /// A scripted transport: each `open` hands out the next reply script and
    /// records every byte written to it. Clones share state.
    #[derive(Clone, Default)]
    pub(crate) struct FakeTransport(Arc<Mutex<State>>);

    struct Recorder(Arc<Mutex<State>>);

    impl Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Notes when a connection is dropped.
    struct CloseMark(Arc<Mutex<State>>, usize);

    impl Drop for CloseMark {
        fn drop(&mut self) {
            if let Ok(mut state) = self.0.lock() {
                state.log.push(Step::Closed(self.1));
            }
        }
    }

    impl FakeTransport {
        pub(crate) fn new(scripts: &[&str]) -> Self {
            let fake = Self::default();
            for script in scripts {
                fake.push(script);
            }
            fake
        }

        /// Queue one more reply script.
        pub(crate) fn push(&self, script: &str) {
            let mut state = self.0.lock().unwrap();
            state
                .scripts
                .push_back(Script::Reply(script.as_bytes().to_vec()));
        }

        /// Queue a reply script whose connection then stays open until it
        /// is closed or dropped.
        pub(crate) fn push_endless(&self, script: &str) {
            let mut state = self.0.lock().unwrap();
            state
                .scripts
                .push_back(Script::Endless(script.as_bytes().to_vec()));
        }

        /// Queue an open that fails the way `SshBridge` reports a failure.
        pub(crate) fn push_failure(&self, failure: SshFailure) {
            self.0
                .lock()
                .unwrap()
                .scripts
                .push_back(Script::Fail(failure));
        }

        pub(crate) fn opens(&self) -> usize {
            self.0.lock().unwrap().opens
        }

        pub(crate) fn log(&self) -> Vec<Step> {
            self.0.lock().unwrap().log.clone()
        }

        /// Every request line written so far, parsed.
        pub(crate) fn requests(&self) -> Vec<serde_json::Value> {
            let state = self.0.lock().unwrap();
            String::from_utf8_lossy(&state.written)
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        }
    }

    impl Transport for FakeTransport {
        fn open(&self) -> io::Result<Conn> {
            let mut state = self.0.lock().unwrap();
            state.opens += 1;
            let (script, endless) = match state.scripts.pop_front() {
                Some(Script::Reply(script)) => (script, false),
                Some(Script::Endless(script)) => (script, true),
                Some(Script::Fail(failure)) => return Err(io::Error::other(failure)),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionRefused,
                        "no script left",
                    ))
                }
            };
            let n = state.conns;
            state.conns += 1;
            state.log.push(Step::Opened(n));
            let cut = Arc::new(Cut::default());
            let closer = cut.clone();
            Ok(Conn {
                writer: Box::new(Recorder(self.0.clone())),
                reader: Box::new(Reader {
                    data: Cursor::new(script),
                    endless,
                    cut,
                }),
                guard: Some(Box::new(CloseMark(self.0.clone(), n))),
                closer: Some(Closer::new(move || {
                    *closer.closed.lock().unwrap() = true;
                    closer.wake.notify_all();
                })),
            })
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn closing_a_local_socket_ends_a_blocked_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let mut conn = LocalSocket(path).open().unwrap();
        // Keep the server end open so only the closer can end the read.
        let _server = listener.accept().unwrap();
        let closer = conn.closer.clone().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            tx.send(conn.reader.read_line(&mut line).map(|_| line))
                .unwrap();
        });
        closer.close();
        let read = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("read returned");
        assert_eq!(read.unwrap(), "");
    }
}
