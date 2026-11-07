use super::super::runs::{
    expand_env, output, set_mirror, start, stop, Mirror, Report, RunEvent, Status,
};
use super::super::Action;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Long enough for a loaded CI runner; every test returns well before it.
const LONG: Duration = Duration::from_secs(30);

/// Held to read a run again after it has finished, which the eviction test
/// could otherwise make it forget.
static FINISHED: RwLock<()> = RwLock::new(());

fn action(id: &str, command: &str, cwd: &Path) -> Action {
    Action {
        id: id.into(),
        label: id.into(),
        detail: None,
        source: "gespan",
        file: ".gespan/tasks.json".into(),
        command: command.into(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
        background: false,
        icon: "play",
        blocked: None,
        note: None,
    }
}

fn trimmed(report: &Report) -> Vec<&str> {
    report.output.iter().map(|line| line.trim()).collect()
}

/// A command that sleeps for about `seconds`.
fn sleeper(seconds: u32) -> String {
    if cfg!(windows) {
        format!("ping -n {} 127.0.0.1 >NUL", seconds + 1)
    } else {
        format!("sleep {seconds}")
    }
}

#[test]
fn a_quick_action_returns_its_exit_code_and_output() {
    let dir = tempfile::tempdir().unwrap();
    let report = start(
        "p",
        &action("test:quick", "echo hello && exit 3", dir.path()),
        LONG,
    );
    assert_eq!(report.status, Status::Exited, "{report:?}");
    assert_eq!(report.exit_code, Some(3));
    assert_eq!(trimmed(&report), ["hello"]);
    assert!(report.run_id.starts_with("run-"), "{}", report.run_id);
}

#[test]
fn it_runs_in_cwd_with_env_expanded_from_gespans_own() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/marker.txt"), "x").unwrap();
    let (list, reference, show) = if cfg!(windows) {
        (
            "dir /b",
            "%CARGO_PKG_NAME%-%GESPAN_RUNS_UNSET%",
            "echo %GREETING%",
        )
    } else {
        (
            "ls",
            "${CARGO_PKG_NAME}-${GESPAN_RUNS_UNSET}",
            "echo $GREETING",
        )
    };
    let mut run = action(
        "test:env",
        &format!("{list} && {show}"),
        &dir.path().join("sub"),
    );
    run.env = vec![("GREETING".into(), format!("hi-{reference}"))];
    let report = start("p", &run, LONG);
    assert_eq!(report.exit_code, Some(0), "{report:?}");
    assert_eq!(trimmed(&report), ["marker.txt", "hi-gespan-core-"]);
}

#[test]
fn a_slow_action_returns_running_then_output_reports_the_exit() {
    let _keep = FINISHED.read().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let command = format!("echo begin && {} && echo end", sleeper(1));
    let begun = Instant::now();
    let first = start(
        "p",
        &action("test:slow", &command, dir.path()),
        Duration::from_millis(200),
    );
    assert!(
        begun.elapsed() < Duration::from_secs(1),
        "{:?}",
        begun.elapsed()
    );
    assert_eq!(first.status, Status::Running, "{first:?}");
    assert_eq!(first.exit_code, None);

    let last = output(&first.run_id, LONG).unwrap();
    assert_eq!(last.run_id, first.run_id);
    assert_eq!(last.status, Status::Exited, "{last:?}");
    assert_eq!(last.exit_code, Some(0));
    // Each line is returned once: what `start` returned isn't repeated.
    let seen: Vec<_> = trimmed(&first).into_iter().chain(trimmed(&last)).collect();
    assert_eq!(seen, ["begin", "end"]);
    // Nothing new after the exit.
    assert_eq!(
        output(&first.run_id, LONG).unwrap().output,
        Vec::<String>::new()
    );
}

#[test]
fn starting_a_running_action_again_returns_the_same_run() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let command = format!("echo waiting && {}", sleeper(30));
    let first = start(
        "p",
        &action("test:again", &command, dir.path()),
        Duration::ZERO,
    );
    let again = start("p", &action("test:again", &command, dir.path()), LONG);
    assert_eq!(first.status, Status::Running);
    assert_eq!(again.run_id, first.run_id);
    assert_eq!(again.status, Status::Running);
    // Another checkout's copy of the action is a run of its own.
    let elsewhere = start(
        "p",
        &action("test:again", &command, other.path()),
        Duration::ZERO,
    );
    assert_ne!(elsewhere.run_id, first.run_id);
    stop(&elsewhere.run_id).unwrap();

    let stopped = stop(&first.run_id).unwrap();
    assert_eq!(stopped.status, Status::Stopped);
    // Once it has stopped, starting it again is a new run.
    let quick = action("test:again", "echo fresh", dir.path());
    let fresh = start("p", &quick, LONG);
    assert_ne!(fresh.run_id, first.run_id);
    assert_eq!(fresh.status, Status::Exited);
}

#[test]
fn output_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let command = if cfg!(windows) {
        "for /L %i in (1,1,1000) do @echo line%i"
    } else {
        "i=1; while [ $i -le 1000 ]; do echo line$i; i=$((i+1)); done"
    };
    let report = start("p", &action("test:bounded", command, dir.path()), LONG);
    assert_eq!(report.status, Status::Exited, "{report:?}");
    assert_eq!(report.output.len(), 200);
    assert_eq!(report.output.first().map(|l| l.trim()), Some("line801"));
    assert_eq!(report.output.last().map(|l| l.trim()), Some("line1000"));
}

#[cfg(unix)]
#[test]
fn a_long_line_is_capped_at_2000_chars() {
    let dir = tempfile::tempdir().unwrap();
    // 3,000 two-byte chars, so a cut by bytes would split one.
    let command = "i=0; while [ $i -lt 3000 ]; do printf 'é'; i=$((i+1)); done; echo";
    let report = start("p", &action("test:long", command, dir.path()), LONG);
    assert_eq!(report.exit_code, Some(0), "{report:?}");
    assert_eq!(report.output.len(), 1, "{:?}", report.output.len());
    assert_eq!(report.output[0].chars().count(), 2_000);
    assert!(report.output[0].chars().all(|c| c == 'é'));
}

/// The pid printed by a run, read as it arrives.
fn printed_pid(run_id: &str) -> u32 {
    let deadline = Instant::now() + LONG;
    while Instant::now() < deadline {
        let report = output(run_id, Duration::from_millis(100)).unwrap();
        if let Some(pid) = report.output.iter().find_map(|l| l.trim().parse().ok()) {
            return pid;
        }
    }
    panic!("no pid printed");
}

fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
    #[cfg(windows)]
    {
        let listed = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&listed.stdout).contains(&format!(" {pid} "))
    }
}

#[test]
fn stop_kills_children() {
    let _keep = FINISHED.read().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let command = if cfg!(windows) {
        "powershell -NoProfile -Command \"$p = Start-Process ping -ArgumentList '-n','60','127.0.0.1' -PassThru -NoNewWindow; $p.Id; $p.WaitForExit()\""
            .to_owned()
    } else {
        format!("{} & echo $!; wait", sleeper(60))
    };
    let run = start(
        "p",
        &action("test:stop", &command, dir.path()),
        Duration::ZERO,
    );
    let child = printed_pid(&run.run_id);
    assert!(alive(child));

    let stopped = stop(&run.run_id).unwrap();
    assert_eq!(stopped.status, Status::Stopped, "{stopped:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(child) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(child), "child {child} outlived stop");
    // Stopping again reports the same end.
    assert_eq!(stop(&run.run_id).unwrap().status, Status::Stopped);
}

#[derive(Default)]
struct Recorder(Mutex<Vec<RunEvent>>);

impl Mirror for Recorder {
    fn event(&self, event: RunEvent) {
        self.0.lock().unwrap().push(event);
    }
}

/// One mirror for every test, as setting one replaces the last.
fn recorder() -> &'static Recorder {
    static RECORDER: LazyLock<Arc<Recorder>> = LazyLock::new(|| {
        let recorder = Arc::new(Recorder::default());
        set_mirror(recorder.clone());
        recorder
    });
    &RECORDER
}

fn run_of(event: &RunEvent) -> &str {
    match event {
        RunEvent::Started { run_id, .. }
        | RunEvent::Output { run_id, .. }
        | RunEvent::Finished { run_id, .. } => run_id,
    }
}

#[test]
fn the_mirror_sees_started_output_finished() {
    let recorder = recorder();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::create_dir(dir.path().join("web")).unwrap();
    let run = start(
        "project-a",
        &action("test:mirror", "echo mirrored", &dir.path().join("web")),
        LONG,
    );
    assert_eq!(run.status, Status::Exited);

    // Events arrive off the run's threads, so the last may still be on its way.
    let deadline = Instant::now() + LONG;
    while Instant::now() < deadline
        && !recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| run_of(event) == run.run_id && matches!(event, RunEvent::Finished { .. }))
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let events = recorder.0.lock().unwrap();
    let ours: Vec<&RunEvent> = events.iter().filter(|e| run_of(e) == run.run_id).collect();
    let Some((
        RunEvent::Started {
            project,
            checkout,
            label,
            command,
            ..
        },
        rest,
    )) = ours.split_first()
    else {
        panic!("not started first");
    };
    assert_eq!(project, "project-a");
    // The checkout holds `.git`; the action runs in a folder of it.
    assert_eq!(checkout, dir.path());
    assert_eq!(label, "test:mirror");
    assert_eq!(command, "echo mirrored");
    let Some((
        RunEvent::Finished {
            status, exit_code, ..
        },
        middle,
    )) = rest.split_last()
    else {
        panic!("not finished last");
    };
    assert_eq!((*status, *exit_code), (Status::Exited, Some(0)));
    let mut bytes = Vec::new();
    for event in middle {
        let RunEvent::Output { bytes: chunk, .. } = event else {
            panic!("only output between start and finish");
        };
        bytes.extend_from_slice(chunk);
    }
    assert_eq!(String::from_utf8_lossy(&bytes).trim(), "mirrored");
}

#[test]
fn an_unknown_run_id_is_an_error() {
    assert!(output("run-unknown", Duration::ZERO).is_err());
    assert!(stop("run-unknown").is_err());
}

#[test]
fn finished_runs_are_forgotten_oldest_first() {
    let _alone = FINISHED.write().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let ids: Vec<String> = (0..40)
        .map(|n| {
            let run = start(
                "p",
                &action(&format!("test:many{n}"), "echo x", dir.path()),
                LONG,
            );
            assert_eq!(run.status, Status::Exited);
            run.run_id
        })
        .collect();
    assert!(output(&ids[0], Duration::ZERO).is_err());
    assert_eq!(
        output(&ids[39], Duration::ZERO).unwrap().status,
        Status::Exited
    );
}

#[test]
fn env_references_expand_for_each_shell() {
    let lookup = |name: &str| match name {
        "HOME" => Some("/home/me".to_owned()),
        _ => None,
    };
    assert_eq!(expand_env("${HOME}/bin", false, lookup), "/home/me/bin");
    assert_eq!(expand_env("a${MISSING}b", false, lookup), "ab");
    assert_eq!(
        expand_env("$HOME ${not valid} ${", false, lookup),
        "$HOME ${not valid} ${"
    );
    assert_eq!(expand_env("%HOME%", false, lookup), "%HOME%");

    assert_eq!(expand_env("%HOME%\\bin", true, lookup), "/home/me\\bin");
    assert_eq!(expand_env("a%MISSING%b", true, lookup), "ab");
    assert_eq!(expand_env("100% sure %", true, lookup), "100% sure %");
    assert_eq!(expand_env("50%%HOME%", true, lookup), "50%/home/me");
    assert_eq!(expand_env("${HOME}", true, lookup), "${HOME}");
}

#[test]
fn stop_kills_what_a_finished_action_left_running() {
    let _keep = FINISHED.read().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let commands = if cfg!(windows) {
        vec![
            "powershell -NoProfile -Command \"(Start-Process ping -ArgumentList '-n','60','127.0.0.1' -PassThru -WindowStyle Hidden).Id\"".to_owned(),
        ]
    } else {
        vec![
            // Holds nothing of the run's: only the process group finds it.
            format!("{} >/dev/null 2>&1 & echo $!", sleeper(60)),
            // Holds the run's output open.
            format!("{} & echo $!", sleeper(60)),
        ]
    };
    for (n, command) in commands.iter().enumerate() {
        let run = start(
            "p",
            &action(&format!("test:left{n}"), command, dir.path()),
            LONG,
        );
        assert_eq!(run.status, Status::Exited, "{run:?}");
        let child: u32 = run
            .output
            .iter()
            .find_map(|l| l.trim().parse().ok())
            .expect("pid printed");
        assert!(alive(child));

        let stopped = stop(&run.run_id).unwrap();
        assert_eq!(stopped.status, Status::Stopped, "{command}: {stopped:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while alive(child) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(child), "{command}: {child} outlived stop");
    }
}

#[test]
fn a_quoted_program_path_with_spaces_runs() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("with space");
    std::fs::create_dir(&folder).unwrap();
    let tool = if cfg!(windows) {
        let tool = folder.join("tool.cmd");
        std::fs::write(&tool, "@echo [%~1]\r\n").unwrap();
        tool
    } else {
        let tool = folder.join("tool.sh");
        std::fs::write(&tool, "#!/bin/sh\nprintf '[%s]\\n' \"$1\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        tool
    };
    let command = format!("\"{}\" \"a b\"", tool.display());
    let report = start("p", &action("test:quoted", &command, dir.path()), LONG);
    assert_eq!(report.exit_code, Some(0), "{report:?}");
    assert_eq!(trimmed(&report), ["[a b]"]);
}

#[test]
fn concurrent_starts_of_one_action_share_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let run = action("test:concurrent", &sleeper(30), dir.path());
    let ids: Vec<String> = std::thread::scope(|scope| {
        let starts: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| start("p", &run, Duration::ZERO).run_id))
            .collect();
        starts.into_iter().map(|s| s.join().unwrap()).collect()
    });
    assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
    stop(&ids[0]).unwrap();
}

/// What a finished action left running may still print. The run keeps it, but
/// the mirror has been told the run finished and hears nothing more.
#[cfg(unix)]
#[test]
fn the_mirror_hears_nothing_after_finished() {
    let _keep = FINISHED.read().unwrap_or_else(|e| e.into_inner());
    let recorder = recorder();
    let dir = tempfile::tempdir().unwrap();
    let command = "(sleep 1; echo late) & echo early";
    let run = start("p", &action("test:late", command, dir.path()), LONG);
    assert_eq!(run.status, Status::Exited, "{run:?}");
    let deadline = Instant::now() + LONG;
    let mut lines = run.output.clone();
    while !lines.iter().any(|l| l == "late") && Instant::now() < deadline {
        lines.extend(
            output(&run.run_id, Duration::from_millis(50))
                .unwrap()
                .output,
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(lines, ["early", "late"]);
    // Time for anything sent to reach the mirror.
    std::thread::sleep(Duration::from_millis(300));
    let events = recorder.0.lock().unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .filter(|e| run_of(e) == run.run_id)
        .map(|e| match e {
            RunEvent::Started { .. } => "started",
            RunEvent::Output { .. } => "output",
            RunEvent::Finished { .. } => "finished",
        })
        .collect();
    assert_eq!(kinds.last(), Some(&"finished"), "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "finished").count(), 1);
}
