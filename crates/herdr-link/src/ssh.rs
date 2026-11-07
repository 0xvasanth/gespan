use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::transport::{Closer, Conn, Transport};
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
/// `remote-api-bridge`. Connections share one ssh master when `control_dir`
/// is usable, so they only authenticate once.
pub struct SshBridge {
    pub target: String,
    pub session: String,
    /// Where the shared control socket lives; `None` means no sharing.
    pub control_dir: Option<PathBuf>,
    pub ssh: PathBuf,
}

impl SshBridge {
    pub fn for_machine(m: &Machine) -> Self {
        Self {
            target: m.target.clone(),
            session: m.session.clone(),
            control_dir: default_control_dir(m),
            ssh: PathBuf::from("ssh"),
        }
    }

    /// The target, if it cannot be mistaken for an ssh option or carry
    /// control characters (same rule as herdr's own ssh commands).
    fn validated_target(&self) -> io::Result<&str> {
        let t = self.target.as_str();
        if t.is_empty() || t.starts_with('-') || t.chars().any(char::is_control) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid SSH target",
            ));
        }
        Ok(t)
    }

    /// The socket to share, if the control directory is safe to use.
    fn shared_socket(&self) -> Option<PathBuf> {
        let dir = self.control_dir.as_deref()?;
        prepare_private_dir(dir).then(|| dir.join("cm"))
    }

    pub fn command(&self) -> io::Result<Command> {
        let target = self.validated_target()?;
        let mut cmd = Command::new(&self.ssh);
        for opt in [
            "BatchMode=yes",
            "StrictHostKeyChecking=yes",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=3",
            "ConnectTimeout=10",
        ] {
            cmd.arg("-o").arg(opt);
        }
        match self.shared_socket() {
            Some(socket) => {
                cmd.args(["-o", "ControlMaster=auto", "-o", "ControlPersist=600", "-S"])
                    .arg(socket);
            }
            None => {
                cmd.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
            }
        }
        cmd.arg("-T").arg("--").arg(target);
        // ssh hands the remote command to the login shell as one string.
        cmd.arg(format!(
            "sh -c {}",
            shell_quote(&remote_script(&self.session))
        ));
        Ok(cmd)
    }
}

/// Make sure `dir` is a private directory we own, creating it if needed. A
/// directory someone else could have planted or can read is never used.
#[cfg(unix)]
fn prepare_private_dir(dir: &Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if let Some(parent) = dir.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return false,
    }
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return false;
    };
    // SAFETY: getuid has no preconditions and cannot fail.
    let me = unsafe { libc::getuid() };
    meta.file_type().is_dir() && meta.uid() == me && meta.mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn prepare_private_dir(_dir: &Path) -> bool {
    false
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A unix socket path must stay under 104 bytes on macOS, and ssh adds a
/// 17 byte suffix while it sets the master up.
const MAX_CONTROL_PATH: usize = 104 - 17;

/// FNV-1a, 64 bit: a stable hash that needs no dependency.
fn fnv1a(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for part in parts {
        for b in part.bytes().chain([0xff]) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// The control directory for a machine under `base`. Named from the machine
/// id and target, so editing the target never reuses the old master.
pub fn control_dir_for(base: &Path, m: &Machine) -> PathBuf {
    base.join(format!("hl-{:016x}", fnv1a(&[&m.id, &m.target])))
}

/// The first base directory whose control socket path is short enough.
fn first_fitting(bases: &[PathBuf], m: &Machine) -> Option<PathBuf> {
    bases
        .iter()
        .map(|base| control_dir_for(base, m))
        .find(|dir| dir.join("cm").as_os_str().len() <= MAX_CONTROL_PATH)
}

/// Where to keep a machine's control socket: `$XDG_RUNTIME_DIR`, the temp
/// dir, or `~/.cache/herdr-link`, whichever fits first. `None` means no
/// sharing.
pub fn default_control_dir(m: &Machine) -> Option<PathBuf> {
    let mut bases = Vec::new();
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        bases.push(PathBuf::from(dir));
    }
    bases.push(std::env::temp_dir());
    if let Some(home) = std::env::var_os("HOME").filter(|d| !d.is_empty()) {
        bases.push(Path::new(&home).join(".cache").join("herdr-link"));
    }
    first_fitting(&bases, m)
}

/// Finds herdr on the remote and execs its API bridge. Mirrors herdr's own
/// discovery in src/remote/attach.rs (posix_remote_api_discovery_script).
pub fn remote_script(session: &str) -> String {
    script_with(
        session,
        &[
            r#""$HOME/.local/bin/herdr""#,
            "/opt/homebrew/bin/herdr",
            "/usr/local/bin/herdr",
        ],
    )
}

/// `candidates` are shell words tried after `command -v herdr`.
fn script_with(session: &str, candidates: &[&str]) -> String {
    let s = shell_quote(session);
    let candidates = candidates.join(" ");
    format!(
        r#"set -f
found=
for c in "$(command -v herdr)" {candidates}; do
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
            .command()?
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
        let child = Arc::new(Mutex::new(child));
        let cut = child.clone();
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
            // Killing ssh ends its stdout, so a blocked read returns. If the
            // reader holds the child it is already reaping it.
            closer: Some(Closer::new(move || {
                if let Ok(mut child) = cut.try_lock() {
                    let _ = child.kill();
                }
            })),
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
    child: Arc<Mutex<Child>>,
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
        let status = lock(&self.child).wait()?;
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
            let mut child = lock(&self.child);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::Client;
    use crate::{Error, Machine};

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
        let mut b = SshBridge::for_machine(&machine());
        b.control_dir = Some(std::env::temp_dir().join("hl-test-base"));
        b
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
        let a = args(&b.command().unwrap());
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
        let socket = b.control_dir.as_ref().unwrap().join("cm");
        let at = a.iter().position(|x| x == "-S").expect("-S given");
        assert_eq!(Path::new(&a[at + 1]), socket);
        assert_eq!(a.iter().filter(|x| *x == "box").count(), 1);
        assert!(a.iter().any(|x| x == "-T"));
    }

    #[test]
    fn command_runs_the_script_as_one_remote_argument() {
        let b = bridge();
        let a = args(&b.command().unwrap());
        let remote = a.last().unwrap();
        assert!(remote.starts_with("sh -c '"), "{remote}");
        assert!(remote.contains("remote-api-bridge --check"));
        // `--` ends options, so the target can never be read as one.
        assert_eq!(&a[a.len() - 3..a.len() - 1], ["--", "box"]);
    }

    fn sharing(a: &[String]) -> bool {
        a.iter().any(|x| x == "-S") && !a.iter().any(|x| x == "ControlPath=none")
    }

    #[cfg(unix)]
    #[test]
    fn fresh_control_dir_is_created_private_and_shared() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let mut b = bridge();
        let dir = base.path().join("a").join("hl-x");
        b.control_dir = Some(dir.clone());
        assert!(sharing(&args(&b.command().unwrap())));
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_control_dir_is_not_shared() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("hl-x");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut b = bridge();
        b.control_dir = Some(dir);
        let a = args(&b.command().unwrap());
        assert!(!sharing(&a), "{a:?}");
        assert!(a.iter().any(|x| x == "ControlMaster=no"));
        assert!(a.iter().any(|x| x == "ControlPath=none"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_control_dir_is_not_shared() {
        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = base.path().join("hl-x");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut b = bridge();
        b.control_dir = Some(link);
        assert!(!sharing(&args(&b.command().unwrap())));
    }

    #[test]
    fn no_control_dir_means_no_sharing() {
        let mut b = bridge();
        b.control_dir = None;
        assert!(!sharing(&args(&b.command().unwrap())));
    }

    #[test]
    fn control_dir_name_is_a_hash_of_id_and_target() {
        let m = machine();
        let d = control_dir_for(Path::new("/tmp/base"), &m);
        let name = d.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 3 + 16);
        assert!(name.starts_with("hl-") && name[3..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(d, control_dir_for(Path::new("/tmp/base"), &m.clone()));
    }

    #[test]
    fn control_dir_changes_with_target_and_with_non_hex_ids() {
        let m = machine();
        let moved = Machine {
            target: "other".into(),
            ..m.clone()
        };
        assert_ne!(
            control_dir_for(Path::new("/t"), &m),
            control_dir_for(Path::new("/t"), &moved)
        );
        // Ids that share no hex characters must not collide.
        let x = Machine {
            id: "ghijk".into(),
            ..m.clone()
        };
        let y = Machine {
            id: "lmnop".into(),
            ..m.clone()
        };
        assert_ne!(
            control_dir_for(Path::new("/t"), &x),
            control_dir_for(Path::new("/t"), &y)
        );
    }

    #[test]
    fn control_path_fits_sun_path() {
        let long = PathBuf::from("/private/tmp/".to_owned() + &"x".repeat(120));
        let short = PathBuf::from("/tmp");
        let dir = first_fitting(&[long, short.clone()], &machine()).unwrap();
        assert!(dir.starts_with(&short), "over-long base falls through");
        assert!(dir.join("cm").as_os_str().len() <= MAX_CONTROL_PATH);
    }

    #[test]
    fn no_candidate_that_fits_means_none() {
        let long = PathBuf::from("/private/tmp/".to_owned() + &"x".repeat(120));
        assert_eq!(first_fitting(&[long.clone(), long], &machine()), None);
    }

    #[test]
    fn default_control_dir_fits_and_names_the_machine() {
        let m = machine();
        let dir = default_control_dir(&m).expect("a candidate fits");
        assert_eq!(
            dir.file_name(),
            control_dir_for(Path::new(""), &m).file_name()
        );
        assert!(dir.join("cm").as_os_str().len() <= MAX_CONTROL_PATH);
    }

    #[test]
    fn hostile_targets_are_refused_before_spawning() {
        for target in ["-oProxyCommand=touch /tmp/pwned", "", "a\nb", "-J x"] {
            let mut b = bridge();
            b.target = target.into();
            // A program that does not exist: spawning would be NotFound.
            b.ssh = PathBuf::from("/nonexistent/ssh");
            assert_eq!(
                b.command().unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{target:?}"
            );
            let err = b.open().err().expect("open refuses");
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{target:?}");
        }
    }

    #[test]
    fn user_at_host_targets_are_allowed() {
        let mut b = bridge();
        b.target = "me@host.example".into();
        assert!(b.command().is_ok());
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
                .arg(script_with("work", &[r#""$HOME/.local/bin/herdr""#]))
                .env_clear()
                .env("HOME", home)
                .env("PATH", path_env)
                .output()
                .unwrap()
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
            let home = tempfile::tempdir().unwrap();
            install(&home.path().join(".local/bin/herdr"), false);
            let out = run(home.path(), "/usr/bin:/bin");
            assert_eq!(out.status.code(), Some(2));
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(map_failure("box", 2, &err), SshFailure::HerdrTooOld);
        }

        #[test]
        fn no_herdr_anywhere_is_missing() {
            let home = tempfile::tempdir().unwrap();
            let out = run(home.path(), "/usr/bin:/bin");
            assert_eq!(out.status.code(), Some(2));
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(map_failure("box", 2, &err), SshFailure::HerdrMissing);
        }

        #[test]
        fn mise_shims_are_never_probed() {
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
            let mut b = SshBridge::for_machine(&machine());
            b.control_dir = Some(dir.join("ctl"));
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
        fn closing_the_connection_kills_ssh_and_ends_a_blocked_read() {
            let dir = tempfile::tempdir().unwrap();
            let b = fake_ssh(dir.path(), "exec sleep 30");
            let mut conn = b.open().unwrap();
            let closer = conn.closer.clone().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut line = String::new();
                let read = conn.reader.read_line(&mut line);
                tx.send(read.map_err(Error::from)).unwrap();
            });
            closer.close();
            let read = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("read returned once ssh was killed");
            // A killed ssh reads as a failed ssh, never as data.
            assert!(!matches!(read, Ok(n) if n > 0), "{read:?}");
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
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let pid = loop {
                if let Ok(s) = std::fs::read_to_string(&marker) {
                    if let Ok(p) = s.trim().parse::<i32>() {
                        break p;
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "fake ssh never wrote its pid"
                );
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
