use std::io::{BufReader, Cursor};
use std::path::PathBuf;

use gespan_speech::protocol::{read_line, write_line, ErrorKind, Event, Request};

fn round_trip<T>(msg: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut buf = Vec::new();
    write_line(&mut buf, msg).unwrap();
    let mut reader = BufReader::new(Cursor::new(buf));
    read_line::<T>(&mut reader).unwrap().expect("one message")
}

fn all_requests() -> Vec<Request> {
    vec![
        Request::Load {
            model_dir: PathBuf::from("/models/parakeet"),
            model_id: "parakeet-tdt-0.6b".into(),
        },
        Request::Start {
            terms: vec!["useInboxUnseen".into(), "Gespan".into()],
        },
        Request::Stop,
        Request::Cancel,
        Request::Shutdown,
    ]
}

fn all_events() -> Vec<Event> {
    vec![
        Event::Ready,
        Event::Loaded,
        Event::Listening,
        Event::Level { rms: 0.25 },
        Event::Partial {
            text: "use inbox".into(),
        },
        Event::Final {
            text: "hi".into(),
            capped: false,
        },
        Event::Cancelled,
        Event::Error {
            kind: ErrorKind::MicrophoneDenied,
            message: "no permission".into(),
        },
    ]
}

#[test]
fn every_request_round_trips() {
    for req in all_requests() {
        assert_eq!(round_trip(&req), req);
    }
}

#[test]
fn every_event_round_trips() {
    for event in all_events() {
        assert_eq!(round_trip(&event), event);
    }
}

#[test]
fn every_error_kind_round_trips() {
    let kinds = [
        ErrorKind::MicrophoneDenied,
        ErrorKind::NoInputDevice,
        ErrorKind::ModelMissing,
        ErrorKind::ModelLoad,
        ErrorKind::Engine,
        ErrorKind::Busy,
    ];
    for kind in kinds {
        let event = Event::Error {
            kind,
            message: String::new(),
        };
        assert_eq!(round_trip(&event), event);
    }
}

#[test]
fn final_has_exact_wire_form() {
    let event = Event::Final {
        text: "hi".into(),
        capped: false,
    };
    let mut buf = Vec::new();
    write_line(&mut buf, &event).unwrap();
    assert_eq!(
        String::from_utf8(buf).unwrap(),
        "{\"type\":\"final\",\"text\":\"hi\",\"capped\":false}\n"
    );
}

#[test]
fn error_kind_uses_snake_case_on_the_wire() {
    let event = Event::Error {
        kind: ErrorKind::MicrophoneDenied,
        message: "denied".into(),
    };
    let mut buf = Vec::new();
    write_line(&mut buf, &event).unwrap();
    let line = String::from_utf8(buf).unwrap();
    assert!(line.contains("\"kind\":\"microphone_denied\""), "{line}");
}

#[test]
fn read_line_skips_blank_lines() {
    let input = b"\n\n{\"type\":\"ready\"}\n\n";
    let mut reader = BufReader::new(Cursor::new(&input[..]));
    assert_eq!(read_line::<Event>(&mut reader).unwrap(), Some(Event::Ready));
    assert_eq!(read_line::<Event>(&mut reader).unwrap(), None);
}

#[test]
fn malformed_line_is_invalid_data() {
    let mut reader = BufReader::new(Cursor::new(&b"{not json}\n"[..]));
    let err = read_line::<Event>(&mut reader).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn unknown_type_is_invalid_data() {
    let mut reader = BufReader::new(Cursor::new(&b"{\"type\":\"explode\"}\n"[..]));
    let err = read_line::<Request>(&mut reader).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn eof_is_none() {
    let mut reader = BufReader::new(Cursor::new(Vec::<u8>::new()));
    assert_eq!(read_line::<Event>(&mut reader).unwrap(), None);
}
