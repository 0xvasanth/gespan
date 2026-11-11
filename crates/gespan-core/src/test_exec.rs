//! Test fixtures that write a script and then run it.
//!
//! Tests run on many threads. When one writes a script with `std::fs::write`,
//! another thread can fork in the moment the write handle is open. The forked
//! child keeps a copy of that handle until it execs, and on Linux running the
//! script meanwhile fails with "Text file busy" (ETXTBSY).
//!
//! [`make_executable`] replaces such a script with a fresh copy written by a
//! separate `cp` process, so this test process never holds a write handle to
//! the file that is run. A handle that leaked into another fork points at the
//! old copy, which nothing executes.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// Make the script at `path` executable with `mode`, as a copy no thread of
/// this process ever opened for writing.
pub fn make_executable(path: &Path, mode: u32) {
    let name = path
        .file_name()
        .expect("script path has a file name")
        .to_string_lossy();
    let staged = path.with_file_name(format!(".{name}.staged"));
    std::fs::rename(path, &staged).expect("stage the script");
    let status = Command::new("cp")
        .arg(&staged)
        .arg(path)
        .stdin(Stdio::null())
        .status()
        .expect("run cp");
    assert!(status.success(), "cp {} failed", path.display());
    std::fs::remove_file(&staged).expect("remove the staged script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .expect("set the script's mode");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    /// A child process that holds a write handle to `path`, the way a fork
    /// from another test thread does until it execs.
    fn leak_write_handle(path: &Path) -> (std::fs::File, std::process::Child) {
        let file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        let fd = file.as_raw_fd();
        let mut command = Command::new("sleep");
        command.arg("30").stdin(Stdio::null());
        // SAFETY: fcntl is async-signal-safe; this only clears close-on-exec
        // so the child keeps the handle across its exec.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        (file, child)
    }

    #[test]
    fn a_script_runs_while_another_process_holds_its_old_write_handle() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("agent");
        let mut file = std::fs::File::create(&script).unwrap();
        file.write_all(b"#!/bin/sh\nprintf ran\n").unwrap();
        drop(file);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (_handle, mut holder) = leak_write_handle(&script);

        // On Linux this is the failure the fixtures used to hit.
        #[cfg(target_os = "linux")]
        {
            let busy = Command::new(&script).output().unwrap_err();
            assert_eq!(busy.kind(), std::io::ErrorKind::ExecutableFileBusy);
        }

        make_executable(&script, 0o700);
        let output = Command::new(&script).output().unwrap();
        holder.kill().unwrap();
        holder.wait().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"ran");
    }

    #[test]
    fn it_keeps_the_content_sets_the_mode_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook");
        std::fs::write(&script, "#!/bin/sh\nexit 3\n").unwrap();
        make_executable(&script, 0o755);
        assert_eq!(
            std::fs::read_to_string(&script).unwrap(),
            "#!/bin/sh\nexit 3\n"
        );
        let mode = std::fs::metadata(&script).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["hook"]);
        assert_eq!(Command::new(&script).status().unwrap().code(), Some(3));
    }
}
