//! Launch sweep ported from src-tauri/src/harness.rs.
//!
//! Every agent CLI is spawned through `process::isolate`, which stamps
//! `GESPAN_HARNESS_PARENT=<our pid>`. A Gespan that crashed or was force-quit
//! leaves those trees running; the next launch finds them by the marker and
//! kills them. A marker whose parent is still alive belongs to a running
//! Gespan (this one, another Native, or the Tauri app) and is left alone.
//! Windows is not swept: there is no cheap way to read another process's
//! environment, as in the Tauri app.
#[cfg(any(unix, test))]
use std::path::Path;

#[cfg(any(unix, test))]
const HARNESS_PARENT_ENV: &str = "GESPAN_HARNESS_PARENT";

/// Off the UI thread: the sweep shells out to `ps` and waits on SIGKILL.
/// Nothing this run spawns can be caught by it, since our own children carry
/// our pid as the marker.
pub fn reap() {
    #[cfg(unix)]
    {
        let our_pid = std::process::id();
        let _ = std::thread::Builder::new()
            .name("orphan-sweep".into())
            .spawn(move || {
                let killed = reap_snapshots(&snapshot_processes(), our_pid);
                if !killed.is_empty() {
                    tracing::info!(count = killed.len(), pids = ?killed, "stopped agents left by an earlier Gespan");
                }
            });
    }
}

#[cfg(any(unix, test))]
#[derive(Debug, Clone)]
struct ProcessSnapshot {
    pid: u32,
    ppid: u32,
    args: String,
    harness_parent: Option<u32>,
}

#[cfg(unix)]
fn reap_snapshots(rows: &[ProcessSnapshot], our_pid: u32) -> Vec<u32> {
    let pids: Vec<u32> = rows
        .iter()
        .filter(|row| should_reap(row, our_pid, process_alive))
        .map(|row| row.pid)
        .collect();
    terminate_all(&pids);
    pids
}

#[cfg(any(unix, test))]
fn should_reap(proc: &ProcessSnapshot, our_pid: u32, parent_alive: impl Fn(u32) -> bool) -> bool {
    if proc.pid == our_pid || proc.pid <= 1 || proc.ppid == our_pid {
        return false;
    }
    // Only marked processes. Tauri also reaped unmarked `cursor-agent acp`
    // under pid 1, left by builds from before the marker; that also matches
    // a Cursor agent another tool started, so Native leaves those alone.
    let Some(parent) = proc.harness_parent else {
        return false;
    };
    looks_like_harness_argv(&proc.args) && parent != our_pid && !parent_alive(parent)
}

/// Argv of an agent CLI we spawned, not a shell, tmux or `npm start` the user
/// ran from a Gespan terminal. The marker still decides what dies.
#[cfg(any(unix, test))]
fn looks_like_harness_argv(args: &str) -> bool {
    args.split_whitespace().any(is_harness_argv_token)
}

#[cfg(any(unix, test))]
fn is_harness_argv_token(part: &str) -> bool {
    let name = Path::new(part)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(part);
    matches!(
        name,
        "cursor-agent"
            | "pi-coding-agent"
            | "claude"
            | "codex"
            | "opencode"
            | "grok"
            | "omp"
            | "fx"
            | "hermes"
            | "kiro-cli"
            | "devin"
            | "agy_acp_server.par"
            | "pi"
            | "worker-server"
            | "app-server"
    )
}

#[cfg(any(all(unix, not(target_os = "linux")), test))]
fn parse_ps_row(line: &str) -> Option<ProcessSnapshot> {
    let s = line.trim();
    let pid_end = s.find(char::is_whitespace)?;
    let pid: u32 = s[..pid_end].parse().ok()?;
    let rest = s[pid_end..].trim_start();
    let ppid_end = rest.find(char::is_whitespace)?;
    let ppid: u32 = rest[..ppid_end].parse().ok()?;
    let args = rest[ppid_end..].trim_start();
    if args.is_empty() {
        return None;
    }
    Some(ProcessSnapshot {
        pid,
        ppid,
        args: args.to_string(),
        harness_parent: harness_parent_from_bytes(args.as_bytes()),
    })
}

#[cfg(any(unix, test))]
fn harness_parent_from_bytes(buf: &[u8]) -> Option<u32> {
    let needle = format!("{HARNESS_PARENT_ENV}=");
    let pos = buf
        .windows(needle.len())
        .position(|chunk| chunk == needle.as_bytes())?;
    let start = pos + needle.len();
    let digits = buf[start..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    std::str::from_utf8(&buf[start..start + digits])
        .ok()?
        .parse()
        .ok()
}

/// Lists argv only, then opens the environment of agent CLIs alone. Linux
/// orphans sit under `systemd --user`, not pid 1, so the marker (not ppid)
/// identifies them.
#[cfg(unix)]
fn snapshot_processes() -> Vec<ProcessSnapshot> {
    #[cfg(target_os = "linux")]
    let mut rows = snapshot_from_proc();
    #[cfg(not(target_os = "linux"))]
    let mut rows = snapshot_from_ps();
    attach_markers_from_env(&mut rows);
    rows
}

/// `ps -E`/`-e` would dump every process environment; skip that here.
#[cfg(all(unix, not(target_os = "linux")))]
fn snapshot_from_ps() -> Vec<ProcessSnapshot> {
    let mut cmd = std::process::Command::new("ps");
    #[cfg(target_os = "macos")]
    cmd.args(["-axww", "-o", "pid=", "-o", "ppid=", "-o", "command="]);
    #[cfg(not(target_os = "macos"))]
    cmd.args(["-axww", "-o", "pid=", "-o", "ppid=", "-o", "args="]);
    let Ok(output) = cmd.output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_ps_row)
        .collect()
}

#[cfg(target_os = "linux")]
fn snapshot_from_proc() -> Vec<ProcessSnapshot> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let dir = entry.path();
            let cmdline = std::fs::read(dir.join("cmdline")).ok()?;
            if cmdline.is_empty() {
                return None;
            }
            Some(ProcessSnapshot {
                pid,
                ppid: parse_proc_ppid(&std::fs::read_to_string(dir.join("stat")).ok()?)?,
                args: String::from_utf8_lossy(&cmdline).replace('\0', " "),
                harness_parent: None,
            })
        })
        .collect()
}

#[cfg(unix)]
fn attach_markers_from_env(rows: &mut [ProcessSnapshot]) {
    let pids: Vec<u32> = rows
        .iter()
        .filter(|row| row.harness_parent.is_none() && looks_like_harness_argv(&row.args))
        .map(|row| row.pid)
        .collect();
    if pids.is_empty() {
        return;
    }
    let parents = read_harness_parents(&pids);
    for row in rows.iter_mut().filter(|row| row.harness_parent.is_none()) {
        row.harness_parent = parents.get(&row.pid).copied();
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn read_harness_parents(pids: &[u32]) -> std::collections::HashMap<u32, u32> {
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut cmd = std::process::Command::new("ps");
    #[cfg(target_os = "macos")]
    cmd.args(["-Eww", "-p", &list, "-o", "pid=", "-o", "command="]);
    #[cfg(not(target_os = "macos"))]
    cmd.args(["-eww", "-p", &list, "-o", "pid=", "-o", "args="]);
    let Ok(output) = cmd.output() else {
        return Default::default();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (pid, command) = line.split_once(char::is_whitespace)?;
            Some((
                pid.parse().ok()?,
                harness_parent_from_bytes(command.as_bytes())?,
            ))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn read_harness_parents(pids: &[u32]) -> std::collections::HashMap<u32, u32> {
    pids.iter()
        .filter_map(|pid| {
            let buf = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
            Some((*pid, harness_parent_from_bytes(&buf)?))
        })
        .collect()
}

/// Field 4 of `stat`, counted from the last closing paren: `comm` is unquoted
/// and can hold spaces and parens of its own.
#[cfg(any(target_os = "linux", test))]
fn parse_proc_ppid(stat: &str) -> Option<u32> {
    stat.get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    pid != 0 && unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Each tree was isolated with `process_group(0)`, so its pid is the group id
/// even after the leader exits.
#[cfg(unix)]
fn tree_alive(pid: u32) -> bool {
    let pid = pid as i32;
    unsafe { libc::kill(pid, 0) == 0 || libc::kill(-pid, 0) == 0 }
}

/// SIGTERM the trees, give them a moment, then SIGKILL what is left.
#[cfg(unix)]
fn terminate_all(pids: &[u32]) {
    use std::time::{Duration, Instant};
    let signal = |sig| {
        for pid in pids.iter().filter(|pid| **pid > 1 && tree_alive(**pid)) {
            unsafe {
                libc::kill(-(*pid as i32), sig);
                libc::kill(*pid as i32, sig);
            }
        }
    };
    let wait = |until: Instant| {
        while Instant::now() < until && pids.iter().any(|pid| tree_alive(*pid)) {
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    if pids.is_empty() {
        return;
    }
    signal(libc::SIGTERM);
    wait(Instant::now() + Duration::from_millis(1500));
    signal(libc::SIGKILL);
    wait(Instant::now() + Duration::from_millis(150));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, ppid: u32, args: &str, parent: Option<u32>) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            ppid,
            args: args.into(),
            harness_parent: parent,
        }
    }

    #[test]
    fn a_marked_agent_whose_gespan_is_gone_is_reaped() {
        let orphan = row(
            500,
            1,
            "/opt/homebrew/bin/claude -p --output-format stream-json",
            Some(400),
        );
        assert!(should_reap(&orphan, 100, |_| false));
    }

    #[test]
    fn agents_of_a_running_gespan_are_spared() {
        // Tauri and Native can run side by side; neither may kill the other's agents.
        let other_app = row(500, 1, "/usr/local/bin/codex app-server", Some(400));
        assert!(!should_reap(&other_app, 100, |pid| pid == 400));
        let ours = row(501, 7, "/usr/local/bin/codex app-server", Some(100));
        assert!(!should_reap(&ours, 100, |_| false));
        let direct_child = row(502, 100, "/usr/local/bin/codex app-server", Some(400));
        assert!(!should_reap(&direct_child, 100, |_| false));
    }

    #[test]
    fn a_marked_process_that_is_not_an_agent_is_spared() {
        // A dev server started from a Gespan terminal inherits the marker.
        let server = row(500, 1, "node /work/app/node_modules/.bin/vite", Some(400));
        assert!(!should_reap(&server, 100, |_| false));
        let shell = row(501, 1, "/bin/zsh -l", Some(400));
        assert!(!should_reap(&shell, 100, |_| false));
    }

    #[test]
    fn an_unmarked_agent_is_never_reaped() {
        // Started by the user or another tool, not by a Gespan.
        let users_own = row(500, 1, "/opt/homebrew/bin/claude", None);
        assert!(!should_reap(&users_own, 100, |_| false));
        let other_tools_cursor = row(501, 1, "/Users/n/.local/bin/cursor-agent acp", None);
        assert!(!should_reap(&other_tools_cursor, 100, |_| false));
    }

    #[test]
    fn init_and_this_process_are_never_reaped() {
        assert!(!should_reap(&row(1, 0, "claude", Some(400)), 100, |_| {
            false
        }));
        assert!(!should_reap(&row(100, 1, "claude", Some(400)), 100, |_| {
            false
        }));
    }

    #[test]
    fn agent_names_match_by_file_name_only() {
        assert!(looks_like_harness_argv("/Users/n/.bun/bin/opencode serve"));
        assert!(looks_like_harness_argv(
            "node /x/agy_acp_server.par --stdio"
        ));
        assert!(!looks_like_harness_argv("/Users/n/claude-notes/run.sh"));
        assert!(!looks_like_harness_argv("vim /work/notes/cursor-agent.md"));
    }

    #[test]
    fn markers_are_read_from_environment_bytes() {
        assert_eq!(
            harness_parent_from_bytes(b"PATH=/bin\0GESPAN_HARNESS_PARENT=4321\0HOME=/h"),
            Some(4321)
        );
        assert_eq!(harness_parent_from_bytes(b"GESPAN_HARNESS_PARENT=\0"), None);
        assert_eq!(harness_parent_from_bytes(b"PATH=/bin"), None);
    }

    #[test]
    fn ps_rows_parse_pid_ppid_and_args() {
        let parsed = parse_ps_row("  812     1 /usr/local/bin/codex app-server --listen").unwrap();
        assert_eq!((parsed.pid, parsed.ppid), (812, 1));
        assert_eq!(parsed.args, "/usr/local/bin/codex app-server --listen");
        assert!(parse_ps_row("812 1").is_none());
        assert!(parse_ps_row("pid ppid command").is_none());
    }

    #[test]
    fn proc_stat_ppid_survives_parens_in_the_command_name() {
        assert_eq!(parse_proc_ppid("812 (node (x) y) S 77 812 812"), Some(77));
        assert_eq!(parse_proc_ppid("garbage"), None);
    }

    #[cfg(unix)]
    mod live {
        use super::*;
        use std::os::unix::process::CommandExt;
        use std::process::{Child, Command, Stdio};

        const DEAD_PARENT: u32 = 2_147_483_646;

        fn spawn(script: &str, marker: u32) -> Child {
            Command::new("sh")
                .args(["-c", script])
                .process_group(0)
                .env(HARNESS_PARENT_ENV, marker.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn test process")
        }

        fn exited(child: &mut Child) -> bool {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while std::time::Instant::now() < until {
                if child.try_wait().ok().flatten().is_some() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        }

        #[test]
        fn an_orphan_that_ignores_sigterm_is_killed() {
            let mut child = spawn("trap '' TERM; while true; do sleep 1; done", DEAD_PARENT);
            let pid = child.id();
            let killed = reap_snapshots(
                &[row(
                    pid,
                    1,
                    "/Users/n/.local/bin/claude -p",
                    Some(DEAD_PARENT),
                )],
                std::process::id(),
            );
            assert_eq!(killed, vec![pid]);
            if !exited(&mut child) {
                let _ = child.kill();
                panic!("the orphan survived the sweep");
            }
        }

        #[test]
        fn a_child_of_this_process_survives_the_sweep() {
            let mut child = spawn("sleep 30", std::process::id());
            let pid = child.id();
            let killed = reap_snapshots(
                &[row(
                    pid,
                    std::process::id(),
                    "/usr/local/bin/codex app-server",
                    Some(std::process::id()),
                )],
                std::process::id(),
            );
            let alive = child.try_wait().ok().flatten().is_none();
            let _ = child.kill();
            let _ = child.wait();
            assert!(killed.is_empty());
            assert!(alive, "the sweep killed a child of this process");
        }

        /// Stand-in agent for the snapshot test; not run on its own.
        #[test]
        #[ignore]
        fn sleeper() {
            std::thread::sleep(std::time::Duration::from_secs(30));
        }

        #[test]
        fn the_real_snapshot_finds_a_marked_orphan_by_its_environment() {
            // macOS hides the environment of Apple-signed binaries such as
            // /bin/sh, so run this test binary under an agent's name. A symlink,
            // not a copy: a copy's write handle can leak into another test's
            // fork and make the exec fail with ETXTBSY on Linux.
            let dir = std::env::temp_dir().join(format!("gespan-orphan-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let fake = dir.join("claude");
            let _ = std::fs::remove_file(&fake);
            std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &fake).unwrap();
            let mut child = Command::new(&fake)
                .args([
                    "--ignored",
                    "--exact",
                    "process::orphans::tests::live::sleeper",
                ])
                .process_group(0)
                .env(HARNESS_PARENT_ENV, DEAD_PARENT.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id();
            // A just-spawned process can show up before its environment does.
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let found = loop {
                let found = snapshot_processes().into_iter().find(|row| row.pid == pid);
                if found
                    .as_ref()
                    .is_some_and(|row| row.harness_parent.is_some())
                    || std::time::Instant::now() > until
                {
                    break found;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            };
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
            let found = found.expect("the snapshot lists the test process");
            assert_eq!(found.harness_parent, Some(DEAD_PARENT));
            // Still our child here; once its Gespan is gone it is reparented.
            assert!(!should_reap(&found, std::process::id(), process_alive));
            let orphaned = ProcessSnapshot { ppid: 1, ..found };
            assert!(should_reap(&orphaned, std::process::id(), process_alive));
        }
    }
}
