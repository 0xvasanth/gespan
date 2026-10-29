//! One Gespan Native per database on Windows, as the Tauri app's
//! single-instance plugin: Close to tray leaves the process running with no
//! taskbar entry, so a second launch brings that window back and exits
//! instead of starting another process on the same database.
//!
//! The lock is an exclusive lock on `<database>.native.lock`, which the OS
//! releases when the process exits, crash included. The holder's process ID
//! is in a separate file: on Windows a locked file can't be read by others.
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// Held for the life of the process.
pub(crate) struct Lock(#[allow(dead_code)] File);

pub(crate) enum Acquired {
    // Read only by `main` on Windows; elsewhere this module runs in tests.
    Lock(#[cfg_attr(not(windows), allow(dead_code))] Lock),
    /// Another process holds the lock; its ID when it could be read.
    Running(Option<u32>),
}

fn paths(database: &Path) -> (PathBuf, PathBuf) {
    let name = database
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gespan.db".into());
    let dir = database.parent().unwrap_or(Path::new("."));
    (
        dir.join(format!("{name}.native.lock")),
        dir.join(format!("{name}.native.pid")),
    )
}

pub(crate) fn acquire(database: &Path) -> std::io::Result<Acquired> {
    let (lock_path, pid_path) = paths(database);
    if let Some(dir) = lock_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    match file.try_lock() {
        Ok(()) => {
            std::fs::write(&pid_path, std::process::id().to_string())?;
            Ok(Acquired::Lock(Lock(file)))
        }
        Err(TryLockError::WouldBlock) => Ok(Acquired::Running(
            std::fs::read_to_string(&pid_path)
                .ok()
                .and_then(|text| text.trim().parse().ok()),
        )),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Show the running instance's main window and bring it to the front.
/// Returns whether one was found.
#[cfg(windows)]
pub(crate) fn activate(pid: u32) -> bool {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetClassNameW, GetWindow, GetWindowTextW, GetWindowThreadProcessId,
    };
    use windows::core::BOOL;

    struct Search {
        pid: u32,
        found: Option<isize>,
    }
    // GPUI's window class, and the main window's title (see `main`).
    const CLASS: &str = "Zed::Window";
    const TITLE: &str = "Gespan Native";

    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
        // SAFETY: `param` is the `Search` passed to EnumWindows below.
        let search = unsafe { &mut *(param.0 as *mut Search) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner != search.pid
            || unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|o| !o.is_invalid())
        {
            return true.into();
        }
        let mut class = [0u16; 64];
        let class_len = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
        let mut title = [0u16; 128];
        let title_len = unsafe { GetWindowTextW(hwnd, &mut title) } as usize;
        if String::from_utf16_lossy(&class[..class_len]) == CLASS
            && String::from_utf16_lossy(&title[..title_len]) == TITLE
        {
            search.found = Some(hwnd.0 as isize);
            return false.into();
        }
        true.into()
    }

    let mut search = Search { pid, found: None };
    // SAFETY: the callback only runs during this call, while `search` lives.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    match search.found {
        Some(hwnd) => {
            crate::tray::set_visible(hwnd, true);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("gespan.db")
    }

    /// One test at a time: a child forked by one test briefly holds copies of
    /// every open file until it execs, so another test's lock can't be freed.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Other tests in this binary fork too, so a released lock can take a
    /// moment to be free; give it a short while.
    fn acquired_soon(db: &Path) -> bool {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if matches!(acquire(db).unwrap(), Acquired::Lock(_)) {
                return true;
            }
            if std::time::Instant::now() > until {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn the_first_launch_holds_the_lock_and_records_its_process() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let held = acquire(&database(&dir)).unwrap();
        assert!(matches!(held, Acquired::Lock(_)));
        let pid = std::fs::read_to_string(dir.path().join("gespan.db.native.pid")).unwrap();
        assert_eq!(pid, std::process::id().to_string());
    }

    #[test]
    fn a_second_launch_finds_the_running_one() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let _held = acquire(&database(&dir)).unwrap();
        match acquire(&database(&dir)).unwrap() {
            Acquired::Running(pid) => assert_eq!(pid, Some(std::process::id())),
            Acquired::Lock(_) => panic!("two launches held the lock at once"),
        }
    }

    #[test]
    fn the_lock_is_free_again_once_the_first_exits() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let held = acquire(&database(&dir)).unwrap();
        drop(held);
        assert!(acquired_soon(&database(&dir)));
    }

    #[test]
    fn separate_databases_do_not_block_each_other() {
        let _serial = serial();
        // A development build and the installed app can run side by side.
        let dir = tempfile::tempdir().unwrap();
        let _release = acquire(&dir.path().join("gespan.db")).unwrap();
        let other = dir.path().join("dev").join("gespan.db");
        assert!(matches!(acquire(&other).unwrap(), Acquired::Lock(_)));
    }

    #[test]
    fn a_missing_process_file_still_reports_a_running_instance() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let _held = acquire(&database(&dir)).unwrap();
        std::fs::remove_file(dir.path().join("gespan.db.native.pid")).unwrap();
        assert!(matches!(
            acquire(&database(&dir)).unwrap(),
            Acquired::Running(None)
        ));
    }

    /// Holds the lock for the database in `GESPAN_INSTANCE_TEST_DB` until
    /// killed; run as a separate process by the tests below.
    #[test]
    #[ignore]
    fn holder() {
        let Some(db) = std::env::var_os("GESPAN_INSTANCE_TEST_DB") else {
            return;
        };
        let _held = acquire(Path::new(&db)).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    fn spawn_holder(db: &Path) -> std::process::Child {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "instance::tests::holder"])
            .env("GESPAN_INSTANCE_TEST_DB", db)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid_file = db.with_file_name("gespan.db.native.pid");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::fs::read_to_string(&pid_file).ok().as_deref() != Some(&child.id().to_string()) {
            assert!(
                std::time::Instant::now() < until,
                "the holder never took the lock"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child
    }

    #[test]
    fn another_process_holding_the_lock_is_found_and_its_crash_frees_it() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let db = database(&dir);
        let mut child = spawn_holder(&db);
        let seen = acquire(&db).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        match seen {
            Acquired::Running(pid) => assert_eq!(pid, Some(child.id())),
            Acquired::Lock(_) => panic!("took the lock while another process held it"),
        }
        // Killed without any cleanup, as in a crash: the OS released the lock.
        assert!(acquired_soon(&db));
    }
}
