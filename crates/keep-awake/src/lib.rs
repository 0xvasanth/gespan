//! Keep the display awake, so the idle timer never locks the screen and pauses
//! a running agent. Shared by the Tauri app and the native app.
//!
//! This only defers the *idle* lock and sleep. A closed lid, a manual lock or a
//! low battery still win; the OS drops the assertion when the process exits.

/// When Rebase holds the keep-awake assertion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Never: the OS idle timer applies as usual.
    #[default]
    Off,
    /// For as long as Rebase is open.
    AppOpen,
    /// Only while at least one agent is working.
    AgentsRunning,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Off, Self::AgentsRunning, Self::AppOpen];

    /// Stored value, identical in the webview's localStorage and native preferences.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::AppOpen => "app_open",
            Self::AgentsRunning => "agents_running",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::AppOpen => "While open",
            Self::AgentsRunning => "While agents run",
        }
    }

    pub fn holds(self, agents_running: bool) -> bool {
        match self {
            Self::Off => false,
            Self::AppOpen => true,
            Self::AgentsRunning => agents_running,
        }
    }
}

/// One OS-level "don't idle" assertion.
pub trait Inhibitor {
    fn acquire(&mut self) -> Result<(), String>;
    fn release(&mut self);
}

/// Holds the assertion exactly while the mode asks for it. A failed acquire
/// is reported once and not retried until the assertion is no longer wanted,
/// so a periodic caller does not retry an unsupported platform on every tick.
pub struct KeepAwake<I: Inhibitor = SystemInhibitor> {
    inhibitor: I,
    held: bool,
    failed: bool,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self::with(SystemInhibitor::default())
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Inhibitor> KeepAwake<I> {
    pub fn with(inhibitor: I) -> Self {
        Self {
            inhibitor,
            held: false,
            failed: false,
        }
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Returns the acquire error only on the transition that failed.
    pub fn update(&mut self, mode: Mode, agents_running: bool) -> Result<(), String> {
        let wanted = mode.holds(agents_running);
        if !wanted {
            self.failed = false;
            if self.held {
                self.inhibitor.release();
                self.held = false;
            }
            return Ok(());
        }
        if self.held || self.failed {
            return Ok(());
        }
        match self.inhibitor.acquire() {
            Ok(()) => {
                self.held = true;
                Ok(())
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }
}

impl<I: Inhibitor> Drop for KeepAwake<I> {
    fn drop(&mut self) {
        if self.held {
            self.inhibitor.release();
        }
    }
}

const REASON: &str = "Rebase is keeping the screen awake so running agents are not paused";

#[cfg(target_os = "macos")]
pub use macos::SystemInhibitor;
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void, CString};

    type CFStringRef = *const c_void;
    const UTF8: u32 = 0x0800_0100;
    const ASSERTION_ON: u32 = 255;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            value: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(value: *const c_void);
    }
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    fn cf_string(value: &str) -> CFStringRef {
        let value = CString::new(value).expect("static string");
        unsafe { CFStringCreateWithCString(std::ptr::null(), value.as_ptr(), UTF8) }
    }

    /// `PreventUserIdleDisplaySleep` keeps the display on, which also keeps
    /// the screen saver and idle lock away and implies no idle system sleep.
    #[derive(Default)]
    pub struct SystemInhibitor {
        id: Option<u32>,
    }

    impl super::Inhibitor for SystemInhibitor {
        fn acquire(&mut self) -> Result<(), String> {
            let kind = cf_string("PreventUserIdleDisplaySleep");
            let name = cf_string(super::REASON);
            let mut id = 0;
            let status = unsafe { IOPMAssertionCreateWithName(kind, ASSERTION_ON, name, &mut id) };
            unsafe {
                CFRelease(kind);
                CFRelease(name);
            }
            if status != 0 {
                return Err(format!("IOPMAssertionCreateWithName failed: {status:#x}"));
            }
            self.id = Some(id);
            Ok(())
        }

        fn release(&mut self) {
            if let Some(id) = self.id.take() {
                unsafe { IOPMAssertionRelease(id) };
            }
        }
    }
}

#[cfg(windows)]
pub use windows::SystemInhibitor;
#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Power::{
        PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired,
        PowerRequestSystemRequired, PowerSetRequest,
    };
    use windows_sys::Win32::System::Threading::{
        POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT,
    };

    /// `POWER_REQUEST_CONTEXT_VERSION` in winnt.h; windows-sys omits it.
    const CONTEXT_VERSION: u32 = 0;

    /// A power request is a handle, not thread state like
    /// `SetThreadExecutionState`, so any thread may set and clear it.
    #[derive(Default)]
    pub struct SystemInhibitor {
        request: Option<HANDLE>,
    }

    // The handle is a kernel object usable from any thread.
    unsafe impl Send for SystemInhibitor {}

    impl super::Inhibitor for SystemInhibitor {
        fn acquire(&mut self) -> Result<(), String> {
            let mut reason: Vec<u16> = super::REASON.encode_utf16().chain([0]).collect();
            let mut context = REASON_CONTEXT {
                Version: CONTEXT_VERSION,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                ..Default::default()
            };
            context.Reason.SimpleReasonString = reason.as_mut_ptr();
            let request = unsafe { PowerCreateRequest(&context) };
            if request.is_null() || request == INVALID_HANDLE_VALUE {
                return Err("PowerCreateRequest failed".into());
            }
            let display = unsafe { PowerSetRequest(request, PowerRequestDisplayRequired) };
            let system = unsafe { PowerSetRequest(request, PowerRequestSystemRequired) };
            if display == 0 || system == 0 {
                unsafe { CloseHandle(request) };
                return Err("PowerSetRequest failed".into());
            }
            self.request = Some(request);
            Ok(())
        }

        fn release(&mut self) {
            if let Some(request) = self.request.take() {
                unsafe {
                    PowerClearRequest(request, PowerRequestDisplayRequired);
                    PowerClearRequest(request, PowerRequestSystemRequired);
                    CloseHandle(request);
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::SystemInhibitor;
#[cfg(target_os = "linux")]
mod linux {
    use std::process::{Child, Command, Stdio};

    /// `systemd-inhibit` holds an idle and sleep lock for its child's
    /// lifetime. The child watches our pid, so a crash cannot leak the lock.
    #[derive(Default)]
    pub struct SystemInhibitor {
        child: Option<Child>,
    }

    impl super::Inhibitor for SystemInhibitor {
        fn acquire(&mut self) -> Result<(), String> {
            let child = Command::new("systemd-inhibit")
                .args([
                    "--what=idle:sleep",
                    "--who=Rebase",
                    &format!("--why={}", super::REASON),
                    "--mode=block",
                    "tail",
                    &format!("--pid={}", std::process::id()),
                    "-f",
                    "/dev/null",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| format!("systemd-inhibit: {error}"))?;
            self.child = Some(child);
            Ok(())
        }

        fn release(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[derive(Default)]
pub struct SystemInhibitor;
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
impl Inhibitor for SystemInhibitor {
    fn acquire(&mut self) -> Result<(), String> {
        Err("keeping the system awake is not supported on this platform".into())
    }
    fn release(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default)]
    struct Log {
        acquired: usize,
        released: usize,
        fail: bool,
    }

    struct Fake(Rc<RefCell<Log>>);
    impl Inhibitor for Fake {
        fn acquire(&mut self) -> Result<(), String> {
            let mut log = self.0.borrow_mut();
            log.acquired += 1;
            if log.fail {
                Err("denied".into())
            } else {
                Ok(())
            }
        }
        fn release(&mut self) {
            self.0.borrow_mut().released += 1;
        }
    }

    fn fake() -> (KeepAwake<Fake>, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        (KeepAwake::with(Fake(log.clone())), log)
    }

    fn counts(log: &Rc<RefCell<Log>>) -> (usize, usize) {
        let log = log.borrow();
        (log.acquired, log.released)
    }

    #[test]
    fn stored_values_round_trip_and_unknown_values_are_rejected() {
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(Mode::parse("always"), None);
        assert_eq!(Mode::parse(""), None);
        assert_eq!(Mode::default(), Mode::Off);
    }

    #[test]
    fn each_mode_holds_only_when_it_should() {
        assert!(!Mode::Off.holds(false));
        assert!(!Mode::Off.holds(true));
        assert!(Mode::AppOpen.holds(false));
        assert!(Mode::AppOpen.holds(true));
        assert!(!Mode::AgentsRunning.holds(false));
        assert!(Mode::AgentsRunning.holds(true));
    }

    #[test]
    fn agents_running_acquires_once_and_releases_when_the_last_agent_stops() {
        let (mut awake, log) = fake();
        awake.update(Mode::AgentsRunning, false).unwrap();
        assert_eq!(counts(&log), (0, 0));
        // Repeated ticks while agents keep running reuse the one assertion.
        for _ in 0..3 {
            awake.update(Mode::AgentsRunning, true).unwrap();
        }
        assert!(awake.is_held());
        assert_eq!(counts(&log), (1, 0));
        awake.update(Mode::AgentsRunning, false).unwrap();
        assert!(!awake.is_held());
        assert_eq!(counts(&log), (1, 1));
        awake.update(Mode::AgentsRunning, false).unwrap();
        assert_eq!(counts(&log), (1, 1));
    }

    #[test]
    fn switching_modes_keeps_or_drops_the_assertion() {
        let (mut awake, log) = fake();
        awake.update(Mode::AppOpen, false).unwrap();
        assert_eq!(counts(&log), (1, 0));
        // Still wanted under the new mode: no release/re-acquire churn.
        awake.update(Mode::AgentsRunning, true).unwrap();
        assert_eq!(counts(&log), (1, 0));
        awake.update(Mode::Off, true).unwrap();
        assert_eq!(counts(&log), (1, 1));
        assert!(!awake.is_held());
    }

    #[test]
    fn a_failed_acquire_is_reported_once_and_retried_after_it_stops_being_wanted() {
        let (mut awake, log) = fake();
        log.borrow_mut().fail = true;
        assert!(awake.update(Mode::AppOpen, false).is_err());
        assert!(awake.update(Mode::AppOpen, false).is_ok());
        assert_eq!(counts(&log), (1, 0));
        assert!(!awake.is_held());
        log.borrow_mut().fail = false;
        awake.update(Mode::Off, false).unwrap();
        awake.update(Mode::AppOpen, false).unwrap();
        assert!(awake.is_held());
        assert_eq!(counts(&log), (2, 0));
    }

    #[test]
    fn dropping_a_held_assertion_releases_it() {
        let (mut awake, log) = fake();
        awake.update(Mode::AppOpen, false).unwrap();
        drop(awake);
        assert_eq!(counts(&log), (1, 1));
        let (awake, log) = fake();
        drop(awake);
        assert_eq!(counts(&log), (0, 0));
    }

    /// Exercises the real IOKit assertion and checks `pmset` sees it.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_assertion_is_visible_to_the_power_manager_until_released() {
        let listed = || {
            let output = std::process::Command::new("pmset")
                .args(["-g", "assertions"])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.lines().any(|line| {
                line.contains(&format!("pid {}(", std::process::id()))
                    && line.contains("PreventUserIdleDisplaySleep")
                    && line.contains(REASON)
            })
        };
        let mut awake = KeepAwake::new();
        awake.update(Mode::AppOpen, false).unwrap();
        assert!(awake.is_held());
        assert!(listed());
        awake.update(Mode::Off, false).unwrap();
        assert!(!listed());
    }
}
