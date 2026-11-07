use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::path::PathBuf;

/// One open connection to a herdr server.
pub struct Conn {
    pub writer: Box<dyn Write + Send>,
    pub reader: Box<dyn BufRead + Send>,
    /// Keeps whatever carries the connection alive (an ssh child process);
    /// dropping it ends the connection.
    pub guard: Option<Box<dyn Send>>,
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
        Ok(Conn {
            writer: Box::new(writer),
            reader: Box::new(io::BufReader::new(stream)),
            guard: None,
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
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    enum Script {
        Reply(Vec<u8>),
        Fail(SshFailure),
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
            let script = match state.scripts.pop_front() {
                Some(Script::Reply(script)) => script,
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
            Ok(Conn {
                writer: Box::new(Recorder(self.0.clone())),
                reader: Box::new(Cursor::new(script)),
                guard: Some(Box::new(CloseMark(self.0.clone(), n))),
            })
        }
    }
}
