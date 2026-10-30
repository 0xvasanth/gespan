use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs one turn against a mock Codex that answers `reply`.
fn turn(name: &str, reply: &str, plan: bool) -> Vec<Row> {
    turn_with(name, reply, plan, "").0
}

/// The same, sending `items` (Codex `item/completed` items as JSON) after
/// the reply. Returns the saved rows and the data folder's generated images.
fn turn_with(name: &str, reply: &str, plan: bool, items: &str) -> (Vec<Row>, Vec<PathBuf>) {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "gespan-runtime-plan-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir_all(&fixture.0).unwrap();
    let binary = fixture.0.join("mock-provider");
    std::fs::write(
        &binary,
        format!(
            r#"#!/usr/bin/env python3
import json, sys
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    value = json.loads(line)
    if value.get('id') == 1:
        send({{'id': 1, 'result': {{}}}})
    elif value.get('id') == 2:
        send({{'id': 2, 'result': {{'thread': {{'id': 'native-plan-test'}}}}}})
    elif value.get('id') == 3:
        send({{'method': 'item/agentMessage/delta', 'params': {{'itemId': 'reply', 'delta': {reply}}}}})
        for item in json.loads({items}):
            send({{'method': 'item/completed', 'params': {{'item': item}}}})
        send({{'method': 'turn/completed', 'params': {{'turn': {{'status': 'completed'}}}}}})
        break
"#,
            reply = serde_json::to_string(reply).unwrap(),
            items = serde_json::to_string(if items.is_empty() { "[]" } else { items }).unwrap(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open_gespan(&fixture.0.join("sessions.db")).unwrap();
    let summary = store.create("codex", fixture.0.to_str().unwrap()).unwrap();
    let session = summary.id;
    let store = Arc::new(Mutex::new(store));
    let worker_store = store.clone();
    let (updates, receive) = async_channel::bounded(64);
    let (_controls, control_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Plan the login".into(),
                settings: Settings {
                    plan,
                    ..Settings::default()
                },
            },
            control_rx,
            worker_store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    let mut shown: Vec<Row> = Vec::new();
    loop {
        let update = receive.recv_blocking().expect("runtime closed early");
        shown.truncate(update.start);
        shown.extend(update.rows);
        if update.finished {
            assert_eq!(update.status, "Ready");
            break;
        }
    }
    worker.join().unwrap();
    let saved = locked(&store).unwrap().load(session).unwrap().rows;
    assert_eq!(shown, saved, "the view gets exactly what was saved");
    let images = std::fs::read_dir(fixture.0.join(gespan_core::generated_image::DIR))
        .map(|dir| dir.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    (saved, images)
}

fn roles(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|row| row.role.as_str()).collect()
}

#[test]
fn a_plan_mode_turn_saves_its_reply_as_a_plan() {
    let rows = turn(
        "mode",
        "# Login plan\n\n1. Add the form\n2. Wire auth",
        true,
    );
    assert_eq!(roles(&rows), ["user", "plan"]);
    assert_eq!(
        rows[1].text,
        "# Login plan\n\n1. Add the form\n2. Wire auth"
    );
}

#[test]
fn an_ordinary_turn_keeps_a_plan_shaped_reply_as_text() {
    let rows = turn("ordinary", "# Done\n\n- fixed a\n- fixed b", false);
    assert_eq!(roles(&rows), ["user", "assistant"]);
}

#[test]
fn a_proposed_plan_is_lifted_from_any_turn() {
    let rows = turn(
        "proposed",
        "Revised.\n<proposed_plan>\n# Plan\n- one\n</proposed_plan>\nOk?",
        false,
    );
    assert_eq!(roles(&rows), ["user", "assistant", "plan", "assistant"]);
    assert_eq!(rows[2].text, "# Plan\n- one");
}

#[test]
fn a_codex_image_is_saved_in_the_data_folder_and_shown_after_the_reply() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
    let items = serde_json::json!([
        {"type":"imageGeneration","id":"img","status":"completed","revisedPrompt":"A fox","result":png},
        {"type":"imageGeneration","id":"img","status":"completed","revisedPrompt":"A fox","result":png},
        {"type":"imageGeneration","id":"bad","status":"completed","result":"R0lGODlh"}
    ])
    .to_string();
    let (rows, files) = turn_with("image", "Here it is.", false, &items);
    assert_eq!(roles(&rows), ["user", "assistant", "image", "error"]);
    assert_eq!(files.len(), 1, "a repeated completion saves one file");
    let image: gespan_core::generated_image::Image = serde_json::from_str(&rows[2].text).unwrap();
    assert_eq!(PathBuf::from(&image.path), files[0]);
    assert_eq!(image.alt.as_deref(), Some("A fox"));
    assert_eq!(
        rows[3].text,
        "Could not save generated image: Generated image data is not a PNG image."
    );
}
