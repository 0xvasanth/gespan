use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::transport::{Conn, Transport};
use crate::Machine;

/// Why a machine could not be reached over ssh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshFailure {
    /// Key auth or the host key needs a human; `hint` is the command to run
    /// once in a terminal.
    NeedsSignIn {
        hint: String,
    },
    Unreachable(String),
    HerdrMissing,
    HerdrTooOld,
}

impl fmt::Display for SshFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SshFailure::NeedsSignIn { hint } => {
                write!(f, "ssh needs a sign-in; run `{hint}` once in a terminal")
            }
            SshFailure::Unreachable(why) => write!(f, "machine unreachable: {why}"),
            SshFailure::HerdrMissing => write!(f, "herdr is not installed on the machine"),
            SshFailure::HerdrTooOld => write!(f, "herdr on the machine is too old to be driven"),
        }
    }
}

impl std::error::Error for SshFailure {}

/// herdr's API on another machine, reached through `ssh` and herdr's own
/// `remote-api-bridge`. Every connection shares one ssh master, so it only
/// authenticates once.
pub struct SshBridge {
    pub target: String,
    pub session: String,
    pub control_dir: PathBuf,
    pub ssh: PathBuf,
}

impl SshBridge {
    pub fn for_machine(m: &Machine, control_dir: PathBuf) -> Self {
        Self {
            target: m.target.clone(),
            session: m.session.clone(),
            control_dir,
            ssh: PathBuf::from("ssh"),
        }
    }

    pub fn command(&self) -> Command {
        create_private_dir(&self.control_dir);
        let mut cmd = Command::new(&self.ssh);
        for opt in [
            "BatchMode=yes",
            "StrictHostKeyChecking=yes",
            "ControlMaster=auto",
            "ControlPersist=600",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=3",
            "ConnectTimeout=10",
        ] {
            cmd.arg("-o").arg(opt);
        }
        cmd.arg("-o").arg(format!(
            "ControlPath={}",
            self.control_dir.join("cm").display()
        ));
        cmd.arg("-T").arg(&self.target);
        // ssh hands the remote command to the login shell as one string.
        cmd.arg(format!(
            "sh -c {}",
            shell_quote(&remote_script(&self.session))
        ));
        cmd
    }
}

fn create_private_dir(dir: &Path) {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    // If this fails ssh still runs, just without connection sharing.
    let _ = builder.create(dir);
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A unix socket path must stay under 104 bytes on macOS, and ssh adds a
/// 17 byte suffix while it sets the master up.
const MAX_CONTROL_PATH: usize = 104 - 17;

/// A short, per-machine directory for the shared ssh control socket.
pub fn control_dir_for(base: &Path, machine_id: &str) -> PathBuf {
    let hex: String = machine_id
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(10)
        .collect();
    let name = format!("hl-{hex}");
    let dir = base.join(&name);
    if cfg!(unix) && dir.join("cm").as_os_str().len() > MAX_CONTROL_PATH {
        return Path::new("/tmp").join(name);
    }
    dir
}

/// Finds herdr on the remote and execs its API bridge. Mirrors herdr's own
/// discovery in src/remote/attach.rs (posix_remote_api_discovery_script).
pub fn remote_script(session: &str) -> String {
    let s = shell_quote(session);
    format!(
        r#"set -f
found=
for c in "$(command -v herdr)" "$HOME/.local/bin/herdr" /opt/homebrew/bin/herdr /usr/local/bin/herdr; do
    case "$c" in
        */mise/shims/herdr) continue ;;
        /*) ;;
        *) continue ;;
    esac
    [ -x "$c" ] || continue
    found=1
    if cap=$("$c" --session {s} remote-api-bridge --check </dev/null 2>/dev/null) && [ "$cap" = herdr-api-bridge-v1 ]; then
        exec "$c" --session {s} remote-api-bridge
    fi
done
if [ -n "$found" ]; then
    printf '%s\n' 'remote Herdr does not support machine API forwarding; update Herdr on this machine' >&2
else
    printf '%s\n' herdr-not-found >&2
fi
exit 2"#
    )
}

fn map_failure(target: &str, code: i32, stderr: &str) -> SshFailure {
    let text = stderr.trim();
    let lower = text.to_lowercase();
    match code {
        255 if [
            "host key verification failed",
            "permission denied",
            "remote host identification",
            "authentication",
        ]
        .iter()
        .any(|p| lower.contains(p)) =>
        {
            SshFailure::NeedsSignIn {
                hint: format!("ssh {target}"),
            }
        }
        2 if lower.contains("herdr-not-found") => SshFailure::HerdrMissing,
        2 if lower.contains("does not support") => SshFailure::HerdrTooOld,
        _ if text.is_empty() => SshFailure::Unreachable(format!("ssh exited with status {code}")),
        _ => SshFailure::Unreachable(text.to_owned()),
    }
}

impl Transport for SshBridge {
    fn open(&self) -> io::Result<Conn> {
        let mut child = self
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let drain = std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut all = sink.lock().unwrap();
                // Only the start matters for diagnosing; keep it bounded.
                if all.len() < 8192 {
                    all.extend_from_slice(&buf[..n]);
                }
            }
        });
        Ok(Conn {
            writer: Box::new(Stdin(stdin)),
            reader: Box::new(Stdout {
                inner: BufReader::new(stdout),
                child,
                target: self.target.clone(),
                stderr: captured,
                drain: Some(drain),
                finished: false,
            }),
            guard: None,
        })
    }
}

/// ssh's stdin. A dead ssh must be reported by the reader with its real
/// cause, so a broken pipe here is not an error of its own.
struct Stdin(ChildStdin);

impl Write for Stdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(buf.len()),
            other => other,
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self.0.flush() {
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            other => other,
        }
    }
}

/// ssh's stdout. At end of stream it waits for ssh, and if ssh failed the
/// read errors with the mapped [`SshFailure`] instead of a bare EOF.
struct Stdout {
    inner: BufReader<ChildStdout>,
    child: Child,
    target: String,
    stderr: Arc<Mutex<Vec<u8>>>,
    drain: Option<JoinHandle<()>>,
    finished: bool,
}

impl Stdout {
    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let status = self.child.wait()?;
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
        if status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned();
        let code = status.code().unwrap_or(-1);
        Err(io::Error::other(map_failure(&self.target, code, &stderr)))
    }
}

impl BufRead for Stdout {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.inner.fill_buf()?.is_empty() {
            self.finish()?;
        }
        self.inner.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

impl Read for Stdout {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.fill_buf()?.len().min(out.len());
        out[..n].copy_from_slice(&self.inner.buffer()[..n]);
        self.inner.consume(n);
        Ok(n)
    }
}

impl Drop for Stdout {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, Error, Machine};

    const ID: &str = "5b800c51aa3f4d2e9c11";

    fn machine() -> Machine {
        Machine {
            id: ID.into(),
            label: "Box".into(),
            target: "box".into(),
            session: "work".into(),
            enabled: true,
        }
    }

    fn bridge() -> SshBridge {
        SshBridge::for_machine(&machine(), std::env::temp_dir().join("hl-test-base"))
    }

    fn args(c: &std::process::Command) -> Vec<String> {
        c.get_args()
            .map(|a: &std::ffi::OsStr| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn for_machine_copies_target_and_session() {
        let b = bridge();
        assert_eq!(b.target, "box");
        assert_eq!(b.session, "work");
        assert_eq!(b.ssh, PathBuf::from("ssh"));
    }

    #[test]
    fn command_shares_one_key_only_connection() {
        let b = bridge();
        let a = args(&b.command());
        for want in [
            "BatchMode=yes",
            "ControlMaster=auto",
            "ControlPersist=600",
            "StrictHostKeyChecking=yes",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=3",
            "ConnectTimeout=10",
        ] {
            assert!(a.iter().any(|x| x == want), "missing {want}: {a:?}");
        }
        assert!(a
            .iter()
            .any(|x| x.starts_with("ControlPath=") && x.ends_with("/cm")));
        assert_eq!(a.iter().filter(|x| *x == "box").count(), 1);
        assert!(a.iter().any(|x| x == "-T"));
    }

    #[test]
    fn command_runs_the_script_as_one_remote_argument() {
        let b = bridge();
        let a = args(&b.command());
        let remote = a.last().unwrap();
        assert!(remote.starts_with("sh -c '"), "{remote}");
        assert!(remote.contains("remote-api-bridge --check"));
        // The target is the last argument before the remote command.
        assert_eq!(a[a.len() - 2], "box");
    }

    #[cfg(unix)]
    #[test]
    fn command_creates_the_control_dir_private() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let mut b = bridge();
        b.control_dir = base.path().join("a").join("hl-x");
        let _ = b.command();
        let mode = std::fs::metadata(&b.control_dir)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn control_path_fits_sun_path() {
        let long = PathBuf::from("/private/tmp/".to_owned() + &"x".repeat(120));
        let p = control_dir_for(&long, ID).join("cm");
        assert!(p.as_os_str().len() < 104);
    }

    #[test]
    fn control_dir_uses_base_when_it_fits() {
        let d = control_dir_for(Path::new("/tmp/base"), ID);
        assert_eq!(d, PathBuf::from("/tmp/base/hl-5b800c51aa"));
    }

    #[test]
    fn control_dir_differs_per_machine() {
        let a = control_dir_for(Path::new("/tmp"), "aaaaaaaaaaaa");
        let b = control_dir_for(Path::new("/tmp"), "bbbbbbbbbbbb");
        assert_ne!(a, b);
    }

    #[test]
    fn remote_script_probes_then_execs_with_session() {
        let s = remote_script("work");
        assert!(s.contains("command -v herdr"));
        assert!(s.contains("$HOME/.local/bin/herdr"));
        assert!(s.contains("remote-api-bridge --check"));
        assert!(s.contains("herdr-api-bridge-v1"));
        assert!(s.contains("--session 'work' remote-api-bridge"));
    }

    #[test]
    fn session_is_shell_quoted() {
        assert!(remote_script("a'b").contains(r"'a'\''b'"));
    }

    #[test]
    fn exit_255_with_host_key_text_needs_sign_in() {
        assert!(matches!(
            map_failure("box", 255, "Host key verification failed."),
            SshFailure::NeedsSignIn { hint } if hint == "ssh box"
        ));
    }

    #[test]
    fn exit_255_permission_denied_needs_sign_in() {
        assert!(matches!(
            map_failure("box", 255, "box: Permission denied (publickey)."),
            SshFailure::NeedsSignIn { .. }
        ));
    }

    #[test]
    fn exit_255_other_is_unreachable() {
        let f = map_failure(
            "box",
            255,
            "ssh: connect to host box port 22: Connection refused\n",
        );
        assert!(
            matches!(&f, SshFailure::Unreachable(m) if m.contains("Connection refused")),
            "{f:?}"
        );
    }

    #[test]
    fn probe_exit_2_maps_to_missing_or_too_old() {
        assert_eq!(
            map_failure("box", 2, "herdr-not-found\n"),
            SshFailure::HerdrMissing
        );
        assert_eq!(
            map_failure(
                "box",
                2,
                "remote Herdr does not support machine API forwarding; update Herdr on this machine\n"
            ),
            SshFailure::HerdrTooOld
        );
    }

    #[test]
    fn unknown_exit_is_unreachable_with_the_code_when_stderr_is_empty() {
        let f = map_failure("box", 7, "");
        assert!(
            matches!(&f, SshFailure::Unreachable(m) if m.contains('7')),
            "{f:?}"
        );
    }

    #[test]
    fn failure_display_is_readable() {
        let s = SshFailure::NeedsSignIn {
            hint: "ssh box".into(),
        }
        .to_string();
        assert!(s.contains("ssh box"));
    }

    #[test]
    fn client_error_from_io_unwraps_an_ssh_failure_only() {
        let e: Error = io::Error::other(SshFailure::HerdrMissing).into();
        assert!(matches!(e, Error::Ssh(SshFailure::HerdrMissing)));
        let e: Error = io::Error::other("plain").into();
        assert!(matches!(e, Error::Io(_)));
    }

    // These run the real generated script under `sh` with a fake HOME.
    #[cfg(unix)]
    mod script {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Output};

        fn install(path: &Path, check_ok: bool) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let body = if check_ok {
                "#!/bin/sh\ncase \"$*\" in *--check*) echo herdr-api-bridge-v1;; *) echo \"bridged $*\";; esac\n"
            } else {
                "#!/bin/sh\nexit 1\n"
            };
            std::fs::write(path, body).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn run(home: &Path, path_env: &str) -> Output {
            Command::new("sh")
                .arg("-c")
                .arg(remote_script("work"))
                .env_clear()
                .env("HOME", home)
                .env("PATH", path_env)
                .output()
                .unwrap()
        }

        fn system_herdr() -> bool {
            ["/opt/homebrew/bin/herdr", "/usr/local/bin/herdr"]
                .iter()
                .any(|p| Path::new(p).exists())
        }

        #[test]
        fn execs_the_first_candidate_that_passes_the_check() {
            let home = tempfile::tempdir().unwrap();
            install(&home.path().join(".local/bin/herdr"), true);
            let out = run(home.path(), "/usr/bin:/bin");
            assert!(out.status.success(), "{out:?}");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                "bridged --session work remote-api-bridge"
            );
        }

        #[test]
        fn a_failing_candidate_is_skipped_for_the_next() {
            let home = tempfile::tempdir().unwrap();
            let bin = tempfile::tempdir().unwrap();
            install(&bin.path().join("herdr"), false);
            install(&home.path().join(".local/bin/herdr"), true);
            let path = format!("{}:/usr/bin:/bin", bin.path().display());
            let out = run(home.path(), &path);
            assert!(out.status.success(), "{out:?}");
            assert!(String::from_utf8_lossy(&out.stdout).contains("bridged"));
        }

        #[test]
        fn a_herdr_that_fails_the_check_is_too_old() {
            if system_herdr() {
                return;
            }
            let home = tempfile::tempdir().unwrap();
            install(&home.path().join(".local/bin/herdr"), false);
            let out = run(home.path(), "/usr/bin:/bin");
            assert_eq!(out.status.code(), Some(2));
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(map_failure("box", 2, &err), SshFailure::HerdrTooOld);
        }

        #[test]
        fn no_herdr_anywhere_is_missing() {
            if system_herdr() {
                return;
            }
            let home = tempfile::tempdir().unwrap();
            let out = run(home.path(), "/usr/bin:/bin");
            assert_eq!(out.status.code(), Some(2));
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(map_failure("box", 2, &err), SshFailure::HerdrMissing);
        }

        #[test]
        fn mise_shims_are_never_probed() {
            if system_herdr() {
                return;
            }
            let home = tempfile::tempdir().unwrap();
            let shims = tempfile::tempdir().unwrap();
            let shim = shims.path().join("mise/shims/herdr");
            install(&shim, true);
            let path = format!("{}:/usr/bin:/bin", shim.parent().unwrap().display());
            let out = run(home.path(), &path);
            assert_eq!(out.status.code(), Some(2), "{out:?}");
            assert!(!String::from_utf8_lossy(&out.stdout).contains("bridged"));
        }

        // A tiny shell script stands in for `ssh`.
        fn fake_ssh(dir: &Path, body: &str) -> SshBridge {
            let path = dir.join("fake-ssh");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut b = SshBridge::for_machine(&machine(), dir.join("ctl"));
            b.ssh = path;
            b
        }

        #[test]
        fn failed_ssh_surfaces_as_needs_sign_in() {
            let dir = tempfile::tempdir().unwrap();
            let b = fake_ssh(
                dir.path(),
                "echo 'box: Permission denied (publickey).' >&2\nexit 255",
            );
            let err = Client::new(b).ping().unwrap_err();
            assert!(
                matches!(&err, Error::Ssh(SshFailure::NeedsSignIn { hint }) if hint == "ssh box"),
                "{err:?}"
            );
        }

        #[test]
        fn writing_after_ssh_died_does_not_hide_the_cause() {
            let dir = tempfile::tempdir().unwrap();
            let b = fake_ssh(dir.path(), "echo herdr-not-found >&2\nexit 2");
            let mut conn = b.open().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
            conn.writer.write_all(b"{}\n").unwrap();
            conn.writer.flush().unwrap();
            let mut line = String::new();
            let err = conn.reader.read_line(&mut line).unwrap_err();
            let err: Error = err.into();
            assert!(
                matches!(err, Error::Ssh(SshFailure::HerdrMissing)),
                "{err:?}"
            );
        }

        #[test]
        fn clean_exit_without_reply_is_plain_closed() {
            let dir = tempfile::tempdir().unwrap();
            let b = fake_ssh(dir.path(), "exit 0");
            assert!(matches!(Client::new(b).ping(), Err(Error::Closed)));
        }

        #[test]
        fn a_working_bridge_answers_a_call() {
            let dir = tempfile::tempdir().unwrap();
            let b = fake_ssh(
                dir.path(),
                "read line\necho '{\"id\":\"ping-0\",\"result\":{\"version\":\"0.9.5\",\"protocol\":1}}'",
            );
            assert_eq!(Client::new(b).ping().unwrap().version, "0.9.5");
        }

        #[test]
        fn dropping_the_connection_stops_ssh() {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("pid");
            let b = fake_ssh(
                dir.path(),
                &format!("echo $$ > {}\nexec sleep 30", marker.display()),
            );
            let conn = b.open().unwrap();
            let pid = loop {
                if let Ok(s) = std::fs::read_to_string(&marker) {
                    if let Ok(p) = s.trim().parse::<i32>() {
                        break p;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            drop(conn);
            let alive = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .unwrap();
            assert!(!alive.status.success(), "ssh child still running");
        }
    }
}
