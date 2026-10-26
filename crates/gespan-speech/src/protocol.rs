//! JSON-lines protocol between the Gespan apps and the speech helper process.
//!
//! The apps write one [`Request`] per line to the helper's stdin, and the
//! helper writes one [`Event`] per line to its stdout.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Load {
        model_dir: PathBuf,
        model_id: String,
    },
    Start {
        terms: Vec<String>,
    },
    Stop,
    Cancel,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Ready,
    Loaded,
    Listening,
    /// Input level, sent at most 20 times per second.
    Level {
        rms: f32,
    },
    Partial {
        text: String,
    },
    Final {
        text: String,
        capped: bool,
    },
    Cancelled,
    Error {
        kind: ErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    MicrophoneDenied,
    NoInputDevice,
    ModelMissing,
    ModelLoad,
    Engine,
    Busy,
}

/// Writes `msg` as one JSON object followed by a newline.
pub fn write_line<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let line = serde_json::to_string(msg).map_err(io::Error::other)?;
    w.write_all(line.as_bytes())?;
    w.write_all(b"\n")?;
    w.flush()
}

/// Reads the next non-blank line and decodes it.
///
/// Returns `Ok(None)` at end of file. A line that is not valid JSON for `T`
/// returns an `InvalidData` error.
pub fn read_line<T: DeserializeOwned>(r: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    serde_json::from_str(line.trim_end())
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}
