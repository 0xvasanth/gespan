//! Windows: agent processes start inside a kill-on-close job that Gespan holds
//! the only handle to, so the OS kills them with Gespan even when it crashes.
//! Ported from src-tauri/src/windows.rs.
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::process::{Child, Command};
use std::sync::OnceLock;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

static JOB: OnceLock<Result<OwnedHandle, i32>> = OnceLock::new();

/// Load DLLs only from the app's folder and System32, not the working
/// directory or PATH. Call first thing at startup. Gespan itself stays
/// outside the job, so relaunches and apps it opens are not tied to it.
pub fn harden_dll_search() -> io::Result<()> {
    use windows_sys::Win32::System::LibraryLoader::{
        SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };
    // SAFETY: plain flag call; affects later DLL loads in this process only.
    if unsafe {
        SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn job() -> io::Result<&'static OwnedHandle> {
    JOB.get_or_init(|| create_job().map_err(|e| e.raw_os_error().unwrap_or(1)))
        .as_ref()
        .map_err(|code| io::Error::from_raw_os_error(*code))
}

fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: the handle is owned from here on; the limits struct is
    // zero-initialised as the API expects.
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = OwnedHandle::from_raw_handle(raw);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
}

fn assign(process: RawHandle) -> io::Result<()> {
    // SAFETY: both handles are live for the call.
    if unsafe { AssignProcessToJobObject(job()?.as_raw_handle(), process) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Start `command` suspended, enroll it in the job, then let it run, so not
/// even a process it starts at once can escape. Keeps `isolate`'s flags.
pub(super) fn spawn(command: &mut Command) -> io::Result<Child> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    };
    job()?;
    command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
    let mut child = command.spawn()?;
    if let Err(error) = assign(child.as_raw_handle()).and_then(|()| resume(child.id())) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
}

/// Stable Rust does not expose a child's primary thread handle. The suspended
/// process has run no code yet, so its only thread is in the OS snapshot.
fn resume(pid: u32) -> io::Result<()> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    // SAFETY: every handle is owned and closed on drop; `entry` is sized.
    unsafe {
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = OwnedHandle::from_raw_handle(raw);
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut found = Thread32First(snapshot.as_raw_handle(), &mut entry);
        while found != 0 {
            if entry.th32OwnerProcessID == pid {
                let raw = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = OwnedHandle::from_raw_handle(raw);
                if ResumeThread(thread.as_raw_handle()) == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            found = Thread32Next(snapshot.as_raw_handle(), &mut entry);
        }
        Err(io::Error::other("The new process has no primary thread"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, TerminateProcess, WaitForSingleObject,
        PROCESS_QUERY_INFORMATION,
    };

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_TIMEOUT: u32 = 258;

    fn ping() -> Command {
        let mut command = Command::new("ping.exe");
        command
            .args(["-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn open(pid: u32, access: u32) -> Option<OwnedHandle> {
        // SAFETY: an owned handle or none.
        let raw = unsafe { OpenProcess(access, 0, pid) };
        (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) })
    }

    #[test]
    fn a_short_command_keeps_its_exit_status() {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", "exit", "7"]);
        assert_eq!(spawn(&mut command).unwrap().wait().unwrap().code(), Some(7));
    }

    #[test]
    fn output_still_reaches_the_pipes() {
        let mut command = Command::new("cmd.exe");
        command
            .args(["/D", "/C", "echo ready"])
            .stdout(Stdio::piped());
        let output = spawn(&mut command).unwrap().wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ready");
    }

    #[test]
    fn dll_search_can_be_hardened() {
        harden_dll_search().unwrap();
    }

    /// Runs as a separate process for the test below; `GESPAN_JOB_TEST` says
    /// which part it plays.
    #[test]
    #[allow(clippy::zombie_processes)]
    fn abrupt_exit_kills_agents_and_what_they_start() {
        const ROLE: &str = "GESPAN_JOB_TEST";
        const FILE: &str = "GESPAN_JOB_TEST_FILE";
        let me = || {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args([
                "--exact",
                "process::job::tests::abrupt_exit_kills_agents_and_what_they_start",
                "--nocapture",
            ]);
            command
        };
        match std::env::var(ROLE).as_deref() {
            // An agent that at once starts a process of its own.
            Ok("agent") => {
                let mut child = ping().spawn().unwrap();
                std::fs::write(std::env::var_os(FILE).unwrap(), child.id().to_string()).unwrap();
                let _ = child.wait();
                return;
            }
            // Gespan: starts an agent through the job and a separate app
            // outside it, then dies without running any cleanup.
            Ok("gespan") => {
                let file = std::env::temp_dir()
                    .join(format!("gespan-job-descendant-{}.pid", std::process::id()));
                let _ = std::fs::remove_file(&file);
                let mut agent = me();
                agent
                    .env(ROLE, "agent")
                    .env(FILE, &file)
                    .stdout(Stdio::null());
                let agent = spawn(&mut agent).unwrap();
                let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
                let descendant: u32 = loop {
                    if let Some(pid) = std::fs::read_to_string(&file)
                        .ok()
                        .and_then(|text| text.parse().ok())
                    {
                        break pid;
                    }
                    assert!(std::time::Instant::now() < until, "the agent never started");
                    std::thread::sleep(std::time::Duration::from_millis(10));
                };
                let _ = std::fs::remove_file(&file);
                let handle = open(descendant, PROCESS_QUERY_INFORMATION).unwrap();
                let mut in_job = 0;
                // SAFETY: both handles are live. CI may run us in a job of its
                // own, so ask about ours specifically.
                assert_ne!(
                    unsafe {
                        IsProcessInJob(
                            handle.as_raw_handle(),
                            job().unwrap().as_raw_handle(),
                            &mut in_job,
                        )
                    },
                    0
                );
                assert_ne!(in_job, 0, "the agent's own process escaped the job");
                let external = ping().spawn().unwrap();
                println!("AGENT_PID={}", agent.id());
                println!("DESCENDANT_PID={descendant}");
                println!("EXTERNAL_PID={}", external.id());
                std::io::stdout().flush().unwrap();
                // SAFETY: ends this process at once, skipping destructors as a crash would.
                unsafe { TerminateProcess(GetCurrentProcess(), 0) };
                unreachable!();
            }
            _ => {}
        }
        let mut gespan = me()
            .env(ROLE, "gespan")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = String::new();
        for line in BufReader::new(gespan.stdout.take().unwrap()).lines() {
            let line = line.unwrap();
            lines.push_str(&line);
            lines.push('\n');
            if line.starts_with("EXTERNAL_PID=") {
                break;
            }
        }
        let _ = gespan.wait();
        let pid = |prefix: &str| -> u32 {
            lines
                .lines()
                .find_map(|line| line.strip_prefix(prefix))
                .unwrap_or_else(|| panic!("no {prefix} in {lines}"))
                .parse()
                .unwrap()
        };
        // An app Gespan opened outside the job outlives it.
        let external = open(pid("EXTERNAL_PID="), SYNCHRONIZE | 0x0001).unwrap();
        // SAFETY: live handle.
        let still_running = unsafe { WaitForSingleObject(external.as_raw_handle(), 0) };
        unsafe {
            TerminateProcess(external.as_raw_handle(), 0);
        }
        assert_eq!(
            still_running, WAIT_TIMEOUT,
            "the outside app died with Gespan"
        );
        // The agent and the process it started die with it.
        for prefix in ["AGENT_PID=", "DESCENDANT_PID="] {
            if let Some(process) = open(pid(prefix), SYNCHRONIZE) {
                // SAFETY: live handle.
                assert_eq!(
                    unsafe { WaitForSingleObject(process.as_raw_handle(), 5000) },
                    0,
                    "{prefix} survived Gespan"
                );
            }
        }
    }
}
