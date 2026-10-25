//! Test-only stand-in for the speech helper. `FAKE_SPEECH_MODE` picks the
//! behaviour: `echo`, `crash-after-start` or `hang`. When `FAKE_SPEECH_LOG`
//! is set, the process appends a `pid <n>` line at start and a `load` line
//! for each `Load` request, so tests can tell restarts apart.

use std::fs::OpenOptions;
use std::io::{self, BufReader, Write};
use std::process::exit;

use gespan_speech::protocol::{read_line, write_line, Event, Request};

fn log(line: &str) {
    if let Ok(path) = std::env::var("FAKE_SPEECH_LOG") {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn main() {
    let mode = std::env::var("FAKE_SPEECH_MODE").unwrap_or_else(|_| "echo".into());
    log(&format!("pid {}", std::process::id()));
    let mut out = io::stdout();
    write_line(&mut out, &Event::Ready).unwrap();
    if mode == "hang" {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }
    let mut input = BufReader::new(io::stdin());
    // A malformed request line is skipped, the same as a blank one.
    loop {
        let req = match read_line::<Request>(&mut input) {
            Ok(Some(req)) => req,
            Ok(None) => break,
            Err(_) => continue,
        };
        let reply = match req {
            Request::Load { .. } => {
                log("load");
                Event::Loaded
            }
            Request::Start { .. } => Event::Listening,
            Request::Stop => Event::Final {
                text: "ok".into(),
                capped: false,
            },
            Request::Cancel => Event::Cancelled,
            Request::Shutdown => exit(0),
        };
        let started = reply == Event::Listening;
        write_line(&mut out, &reply).unwrap();
        if started && mode == "crash-after-start" {
            exit(3);
        }
    }
}
