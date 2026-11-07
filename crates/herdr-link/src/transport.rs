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
pub(crate) use fake::FakeTransport;

#[cfg(test)]
mod fake {
    use super::*;
    use std::collections::VecDeque;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct State {
        scripts: VecDeque<Vec<u8>>,
        opens: usize,
        written: Vec<u8>,
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

    impl FakeTransport {
        pub(crate) fn new(scripts: &[&str]) -> Self {
            let state = State {
                scripts: scripts.iter().map(|s| s.as_bytes().to_vec()).collect(),
                ..State::default()
            };
            Self(Arc::new(Mutex::new(state)))
        }

        pub(crate) fn opens(&self) -> usize {
            self.0.lock().unwrap().opens
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
            let script = state.scripts.pop_front().ok_or_else(|| {
                io::Error::new(io::ErrorKind::ConnectionRefused, "no script left")
            })?;
            Ok(Conn {
                writer: Box::new(Recorder(self.0.clone())),
                reader: Box::new(Cursor::new(script)),
                guard: None,
            })
        }
    }
}
