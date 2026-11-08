//! The Gespan browser (`BrowserMode::Managed`): an installed Chromium
//! browser (Chrome, Edge, Brave, Vivaldi, Opera or Arc) started by Gespan with a profile of its own, so agents get a browser
//! without the user's remote-debugging setup or consent dialog.
//!
//! The browser runs with `--remote-debugging-port=0` on its own profile
//! folder (Chrome 136+ refuses that switch only for the default profile) and
//! writes `DevToolsActivePort` there; the link then connects exactly as it
//! does to the user's browser. Logins made in the kept profile stay on disk;
//! the private Gespan browser (`BrowserMode::Private`) gets a fresh profile
//! per start, deleted once it has exited.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::discovery::{self, ActivePort, Candidate, Os, ACTIVE_PORT_FILE};
use super::protocol::{BrowserChoice, BrowserKind, BrowserMode};

/// How long a freshly started browser may take to open its debugging port.
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(100);
/// Channel id reported for the Gespan browser in `BrowserInfo`.
pub const CHANNEL: &str = "gespan";

/// An installed browser Gespan can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Executable {
    pub kind: BrowserKind,
    pub path: PathBuf,
}

/// The profile folder of the Gespan browser.
pub fn profile_dir(app_data: &Path) -> PathBuf {
    app_data.join("browser-profile")
}

/// A fresh profile folder for one start of the private Gespan browser,
/// next to the kept profile.
pub fn private_dir(profile: &Path) -> PathBuf {
    private_root(profile).join(uuid::Uuid::new_v4().simple().to_string())
}

fn private_root(profile: &Path) -> PathBuf {
    profile.parent().unwrap_or(profile).join("browser-private")
}

/// Written into the profile at start: the process id of the browser Gespan
/// started on it, so the next run can stop one that a crash left running.
const PID_FILE: &str = "gespan-browser.pid";

/// Startup: private profiles a crash left behind, and the browsers still
/// running on them (a headless one never exits by itself).
pub fn clear_private(profile: &Path) {
    let root = private_root(profile);
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            stop_orphan(&entry.path());
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

/// Stop the browser an earlier run started on `profile`, when the recorded
/// process still runs on exactly that profile: a reused process id never
/// belongs to another program or to the user's own browser.
#[cfg(unix)]
fn stop_orphan(profile: &Path) {
    let Some(pid) = std::fs::read_to_string(profile.join(PID_FILE))
        .ok()
        .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
        .filter(|pid| *pid > 1)
    else {
        return;
    };
    if !runs_on_profile(pid, profile) {
        return;
    }
    let alive = || unsafe { libc::kill(pid, 0) } == 0;
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let deadline = Instant::now() + ORPHAN_WAIT;
    while alive() && Instant::now() < deadline {
        std::thread::sleep(POLL);
    }
    if alive() {
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let deadline = Instant::now() + ORPHAN_WAIT;
        while alive() && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
    }
}

/// Windows needs nothing here: [`start`] puts the browser in the app's
/// kill-on-close job, so the system ends it when the app's process ends.
#[cfg(not(unix))]
fn stop_orphan(_profile: &Path) {}

/// How long a browser from an earlier run gets to exit after each signal.
#[cfg(unix)]
const ORPHAN_WAIT: Duration = Duration::from_secs(3);

/// Whether process `pid` was started with `--user-data-dir=<profile>`.
#[cfg(unix)]
fn runs_on_profile(pid: libc::pid_t, profile: &Path) -> bool {
    let Ok(output) = Command::new("ps")
        .args(["-ww", "-o", "args=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .contains(&format!("--user-data-dir={}", profile.display()))
}

/// The discovery candidate for the Gespan profile, so a browser that is
/// still running from earlier is found the same way as the user's.
pub fn candidate(profile: &Path, kind: BrowserKind) -> Candidate {
    Candidate {
        channel: CHANNEL.into(),
        kind,
        user_data_dir: profile.to_path_buf(),
    }
}

/// The live endpoint of a Gespan browser that already runs, if any.
pub fn running(profile: &Path) -> Option<ActivePort> {
    let active = discovery::read_active_port(profile)?;
    discovery::port_accepts(active.port, discovery::PROBE_TIMEOUT).then_some(active)
}

/// Executables to try for `choice`, in order. `Auto` and a custom profile
/// folder look for Chrome, then Edge, then the other Chromium browsers; a
/// channel looks for that browser.
fn wanted(choice: &BrowserChoice) -> Vec<BrowserKind> {
    match choice {
        BrowserChoice::Channel { channel } => vec![discovery::channel_kind(channel)],
        BrowserChoice::Auto | BrowserChoice::UserDataDir { .. } => AUTO.to_vec(),
    }
}

const AUTO: [BrowserKind; 6] = [
    BrowserKind::Chrome,
    BrowserKind::Edge,
    BrowserKind::Brave,
    BrowserKind::Vivaldi,
    BrowserKind::Opera,
    BrowserKind::Arc,
];

/// Well-known install locations per platform. `env` reads environment
/// variables (a seam for tests).
pub fn known_paths(
    kind: BrowserKind,
    os: Os,
    env: impl Fn(&str) -> Option<PathBuf>,
) -> Vec<PathBuf> {
    match os {
        Os::Windows => {
            let relatives: &[&[&str]] = match kind {
                BrowserKind::Edge => &[&["Microsoft", "Edge", "Application", "msedge.exe"]],
                BrowserKind::Chrome | BrowserKind::Chromium => {
                    &[&["Google", "Chrome", "Application", "chrome.exe"]]
                }
                BrowserKind::Brave => {
                    &[&["BraveSoftware", "Brave-Browser", "Application", "brave.exe"]]
                }
                BrowserKind::Vivaldi => &[&["Vivaldi", "Application", "vivaldi.exe"]],
                // The per-user install lives under Local\Programs; opera.exe
                // is a launcher that passes the switches on.
                BrowserKind::Opera => {
                    &[&["Programs", "Opera", "opera.exe"], &["Opera", "opera.exe"]]
                }
                // Arc for Windows is a store package with no stable path.
                BrowserKind::Arc => &[],
            };
            ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
                .iter()
                .filter_map(|var| env(var))
                .flat_map(|base| {
                    relatives.iter().map(move |relative| {
                        relative
                            .iter()
                            .fold(base.clone(), |path, part| path.join(part))
                    })
                })
                .collect()
        }
        Os::Mac => {
            // Stable first, then the test builds.
            let apps: &[&str] = match kind {
                BrowserKind::Edge => &[
                    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                    "Microsoft Edge Beta.app/Contents/MacOS/Microsoft Edge Beta",
                    "Microsoft Edge Dev.app/Contents/MacOS/Microsoft Edge Dev",
                    "Microsoft Edge Canary.app/Contents/MacOS/Microsoft Edge Canary",
                ],
                BrowserKind::Chrome => &[
                    "Google Chrome.app/Contents/MacOS/Google Chrome",
                    "Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
                    "Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
                    "Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
                ],
                BrowserKind::Chromium => &["Chromium.app/Contents/MacOS/Chromium"],
                BrowserKind::Brave => &["Brave Browser.app/Contents/MacOS/Brave Browser"],
                BrowserKind::Vivaldi => &["Vivaldi.app/Contents/MacOS/Vivaldi"],
                BrowserKind::Opera => &["Opera.app/Contents/MacOS/Opera"],
                BrowserKind::Arc => &["Arc.app/Contents/MacOS/Arc"],
            };
            let home = env("HOME");
            apps.iter()
                .flat_map(|app| {
                    let system = Path::new("/Applications").join(app);
                    let user = home
                        .as_ref()
                        .map(|home| home.join("Applications").join(app));
                    std::iter::once(system).chain(user)
                })
                .collect()
        }
        Os::Linux => {
            // Distribution packages only: Flatpak and Snap browsers are sandboxed
            // and cannot use the profile folder Gespan passes (`is_sandboxed`).
            let names: &[&str] = match kind {
                BrowserKind::Edge => &[
                    "microsoft-edge",
                    "microsoft-edge-stable",
                    "microsoft-edge-beta",
                    "microsoft-edge-dev",
                ],
                BrowserKind::Chrome => &[
                    "google-chrome",
                    "google-chrome-stable",
                    "google-chrome-beta",
                    "google-chrome-unstable",
                ],
                BrowserKind::Chromium => &["chromium", "chromium-browser"],
                BrowserKind::Brave => &["brave-browser", "brave-browser-stable", "brave"],
                BrowserKind::Vivaldi => &["vivaldi", "vivaldi-stable"],
                BrowserKind::Opera => &["opera"],
                BrowserKind::Arc => &[],
            };
            let dirs: Vec<PathBuf> = env("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default();
            names
                .iter()
                .flat_map(|name| dirs.iter().map(move |dir| dir.join(name)))
                .collect()
        }
    }
}

/// Flatpak and Snap launchers run in a sandbox that cannot use the profile
/// folder Gespan hands the browser, so starting one only times out. They are
/// found for "Your own browser" (profile folders), never started.
fn is_sandboxed(path: &Path) -> bool {
    path.starts_with("/snap") || path.components().any(|part| part.as_os_str() == "flatpak")
}

/// Every well-known launcher of every supported browser for the user whose
/// home folder is `home`, in the order [`find_executable`] prefers them. Pure
/// over `home` and `os`, so every layout is testable on any host.
///
/// Windows: `%LOCALAPPDATA%` is taken as `home\AppData\Local` and the
/// Program Files folders as `C:\Program Files` and `C:\Program Files (x86)`
/// (the usual values). [`find_executable`] additionally asks the real
/// environment, so relocated folders are still found. Linux: the system
/// `bin` folders rather than the live `PATH`; Snap and Flatpak launchers are
/// left out on purpose (see `is_sandboxed`).
pub fn install_paths(home: &Path, os: Os) -> Vec<(BrowserKind, PathBuf)> {
    let env = |var: &str| match var {
        "HOME" => Some(home.to_path_buf()),
        "LOCALAPPDATA" => Some(home.join("AppData").join("Local")),
        "ProgramFiles" => Some(PathBuf::from(r"C:\Program Files")),
        "ProgramFiles(x86)" => Some(PathBuf::from(r"C:\Program Files (x86)")),
        "PATH" => std::env::join_paths(["/usr/bin", "/usr/local/bin"])
            .ok()
            .map(PathBuf::from),
        _ => None,
    };
    [AUTO.as_slice(), &[BrowserKind::Chromium]]
        .concat()
        .into_iter()
        .flat_map(|kind| {
            known_paths(kind, os, env)
                .into_iter()
                .map(move |path| (kind, path))
        })
        .collect()
}

/// The current user's home folder.
fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from)
}

/// Command-line switches that depend on the operating system and on whether
/// Gespan starts the browser. An agent drives tabs that are hidden, covered
/// or in a background window, which Chromium would otherwise throttle, so the
/// browsers Gespan starts (`Managed`, `Private`) run with throttling off; on
/// Linux the OS keyring prompt is skipped (`--password-store=basic`) because
/// the profile is Gespan's own. `Attach` starts nothing, so it has no
/// switches: the user's own browser keeps its own settings.
pub fn flags(mode: BrowserMode, os: Os) -> Vec<String> {
    if !mode.launches() {
        return Vec::new();
    }
    let mut flags: Vec<String> = [
        "--disable-background-timer-throttling",
        "--disable-renderer-backgrounding",
        "--disable-backgrounding-occluded-windows",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if os == Os::Linux {
        flags.push("--password-store=basic".into());
    }
    flags
}

/// The first installed browser for `choice`.
pub fn find_executable(choice: &BrowserChoice) -> Option<Executable> {
    let os = Os::current();
    let extra = home_dir()
        .map(|home| install_paths(&home, os))
        .unwrap_or_default();
    let mut kinds = wanted(choice);
    // Linux distributions ship Chromium itself; it comes right after Chrome
    // and Edge.
    if os == Os::Linux {
        if let Some(at) = kinds
            .iter()
            .rposition(|kind| matches!(kind, BrowserKind::Chrome | BrowserKind::Edge))
        {
            kinds.insert(at + 1, BrowserKind::Chromium);
        }
    }
    kinds.into_iter().find_map(|kind| {
        known_paths(kind, os, |var| std::env::var_os(var).map(PathBuf::from))
            .into_iter()
            .chain(
                extra
                    .iter()
                    .filter(|(extra_kind, _)| *extra_kind == kind)
                    .map(|(_, path)| path.clone()),
            )
            .find(|path| path.is_file() && !is_sandboxed(path))
            .map(|path| Executable { kind, path })
    })
}

/// Command-line switches for the Gespan browser. `scale` is the
/// display's pixel ratio: a headless browser has no screen of its own and
/// would paint at 1, so pictures of it shown in Gespan at 125 % or 150 %
/// would be stretched and soft; it paints at Gespan's ratio instead.
pub fn arguments(profile: &Path, show_window: bool, scale: Option<f64>) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ];
    if !show_window {
        args.push("--headless=new".into());
        // No scrollbars at all, rather than hidden ones that still take
        // their width: the page's layout viewport is then the picture's,
        // which the live pane sizes the tab's window by.
        args.push("--hide-scrollbars".into());
        if let Some(scale) = scale.filter(|s| s.is_finite() && *s > 1.0 && *s <= 4.0) {
            args.push(format!("--force-device-scale-factor={scale}"));
        }
    }
    // Both browsers Gespan starts get the same switches; `Managed` stands
    // for either.
    args.extend(flags(BrowserMode::Managed, Os::current()));
    args.push("about:blank".into());
    args
}

/// Start the browser. A leftover `DevToolsActivePort` from an earlier run
/// goes first, so [`wait_for_endpoint`] only sees the new one. The host
/// hands the process to [`crate::Embedder::browser_started`] (on Windows the
/// app puts it in its job, so it ends with the app even after a crash).
pub fn start(
    exe: &Executable,
    profile: &Path,
    show_window: bool,
    scale: Option<f64>,
) -> std::io::Result<Child> {
    std::fs::create_dir_all(profile)?;
    let _ = std::fs::remove_file(profile.join(ACTIVE_PORT_FILE));
    let mut command = Command::new(&exe.path);
    command
        .args(arguments(profile, show_window, scale))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = command.spawn()?;
    let _ = std::fs::write(profile.join(PID_FILE), child.id().to_string());
    Ok(child)
}

/// The started browser process, as [`wait_for_endpoint`] polls it.
pub enum Started {
    Running,
    Exited(String),
    /// No longer this start's to wait for (OFF, a mode change).
    Gone,
}

/// After the started process exited: how long another process on the same
/// profile may take to come up. Chrome hands a second start of a running
/// profile to the first process and exits at once.
const HANDOVER_GRACE: Duration = Duration::from_secs(2);

/// Wait until the started browser answers on its debugging port, or it
/// exits (plus [`HANDOVER_GRACE`]), or is gone, or `timeout` passes.
pub fn wait_for_endpoint(
    profile: &Path,
    timeout: Duration,
    mut started: impl FnMut() -> Started,
) -> Result<ActivePort, String> {
    let deadline = Instant::now() + timeout;
    let mut exited: Option<(String, Instant)> = None;
    loop {
        if let Some(active) = running(profile) {
            return Ok(active);
        }
        if exited.is_none() {
            match started() {
                Started::Running => {}
                Started::Exited(status) => {
                    exited = Some((status, (Instant::now() + HANDOVER_GRACE).min(deadline)));
                }
                Started::Gone => return Err("The browser start was cancelled.".into()),
            }
        }
        if let Some((status, until)) = &exited {
            if Instant::now() >= *until {
                return Err(format!("The browser exited ({status})."));
            }
        }
        if Instant::now() >= deadline {
            return Err("The browser did not open its debugging port in time.".into());
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_cover_both_program_files_and_the_user_install() {
        let env = |var: &str| match var {
            "ProgramFiles" => Some(PathBuf::from(r"C:\Program Files")),
            "ProgramFiles(x86)" => Some(PathBuf::from(r"C:\Program Files (x86)")),
            "LOCALAPPDATA" => Some(PathBuf::from(r"C:\Users\o\AppData\Local")),
            _ => None,
        };
        let chrome = known_paths(BrowserKind::Chrome, Os::Windows, env);
        assert_eq!(chrome.len(), 3);
        assert_eq!(
            chrome[0],
            PathBuf::from(r"C:\Program Files")
                .join("Google")
                .join("Chrome")
                .join("Application")
                .join("chrome.exe")
        );
        assert!(chrome[2].starts_with(r"C:\Users\o\AppData\Local"));
        let edge = known_paths(BrowserKind::Edge, Os::Windows, env);
        assert!(edge[1].ends_with(
            PathBuf::from("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe")
        ));
    }

    #[test]
    fn mac_and_linux_paths() {
        let env = |var: &str| match var {
            "HOME" => Some(PathBuf::from("/Users/o")),
            "PATH" => Some(PathBuf::from("/usr/bin")),
            _ => None,
        };
        let mac = known_paths(BrowserKind::Edge, Os::Mac, env);
        assert_eq!(
            mac[0],
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge")
        );
        // Stable, Beta, Dev and Canary, each in /Applications and ~/Applications.
        assert_eq!(mac.len(), 8);
        assert_eq!(
            mac[1],
            PathBuf::from("/Users/o/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge")
        );
        let linux = known_paths(BrowserKind::Chrome, Os::Linux, env);
        assert_eq!(linux[0], PathBuf::from("/usr/bin/google-chrome"));
        assert_eq!(
            known_paths(BrowserKind::Arc, Os::Mac, env)[0],
            PathBuf::from("/Applications/Arc.app/Contents/MacOS/Arc")
        );
        assert_eq!(
            known_paths(BrowserKind::Brave, Os::Linux, env)[0],
            PathBuf::from("/usr/bin/brave-browser")
        );
        assert!(known_paths(BrowserKind::Arc, Os::Linux, env).is_empty());
    }

    #[test]
    fn windows_paths_of_the_other_chromium_browsers() {
        let env = |var: &str| match var {
            "ProgramFiles" => Some(PathBuf::from(r"C:\Program Files")),
            "LOCALAPPDATA" => Some(PathBuf::from(r"C:\Users\o\AppData\Local")),
            _ => None,
        };
        let opera = known_paths(BrowserKind::Opera, Os::Windows, env);
        assert!(opera.contains(
            &PathBuf::from(r"C:\Users\o\AppData\Local")
                .join("Programs")
                .join("Opera")
                .join("opera.exe")
        ));
        let brave = known_paths(BrowserKind::Brave, Os::Windows, env);
        assert!(brave[0].ends_with(PathBuf::from("Application").join("brave.exe")));
        let vivaldi = known_paths(BrowserKind::Vivaldi, Os::Windows, env);
        assert!(vivaldi[1].starts_with(r"C:\Users\o\AppData\Local"));
        assert!(known_paths(BrowserKind::Arc, Os::Windows, env).is_empty());
    }

    #[test]
    fn macos_finds_chrome_beta_and_canary() {
        let paths = install_paths(Path::new("/Users/o"), Os::Mac);
        for app in ["Google Chrome Beta", "Google Chrome Canary"] {
            let rel = format!("{app}.app/Contents/MacOS/{app}");
            for root in ["/Applications", "/Users/o/Applications"] {
                assert!(
                    paths.contains(&(BrowserKind::Chrome, Path::new(root).join(&rel))),
                    "missing {root}/{rel}"
                );
            }
        }
        // Stable stays ahead of the test builds.
        let chrome: Vec<&PathBuf> = paths
            .iter()
            .filter(|(kind, _)| *kind == BrowserKind::Chrome)
            .map(|(_, path)| path)
            .collect();
        assert!(chrome[0].ends_with("Google Chrome.app/Contents/MacOS/Google Chrome"));
        assert!(paths.contains(&(
            BrowserKind::Chromium,
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium")
        )));
        assert!(paths.contains(&(
            BrowserKind::Arc,
            PathBuf::from("/Applications/Arc.app/Contents/MacOS/Arc")
        )));
    }

    #[test]
    fn sandboxed_launchers_are_never_started() {
        let home = Path::new("/home/o");
        let paths = install_paths(home, Os::Linux);
        for (kind, path) in [
            (BrowserKind::Chromium, "/usr/bin/chromium"),
            (BrowserKind::Chrome, "/usr/bin/google-chrome-beta"),
        ] {
            assert!(paths.contains(&(kind, PathBuf::from(path))), "{path}");
        }
        assert!(
            !paths.iter().any(|(_, path)| {
                let text = path.to_string_lossy();
                text.contains("flatpak") || text.starts_with("/snap")
            }),
            "{paths:?}"
        );
        assert!(!paths.iter().any(|(kind, _)| *kind == BrowserKind::Arc));
        // Their profiles are still found for "Your own browser".
        let profiles = discovery::profile_dirs(home, Os::Linux);
        assert!(profiles.contains(&(
            BrowserKind::Chrome,
            PathBuf::from("/home/o/.var/app/com.google.Chrome/config/google-chrome")
        )));
        assert!(profiles.contains(&(
            BrowserKind::Chromium,
            PathBuf::from("/home/o/snap/chromium/common/chromium")
        )));
        // A snap or flatpak launcher on the user's PATH is not a start candidate.
        assert!(is_sandboxed(Path::new("/snap/bin/chromium")));
        assert!(is_sandboxed(Path::new(
            "/var/lib/flatpak/exports/bin/com.google.Chrome"
        )));
        assert!(is_sandboxed(Path::new(
            "/home/o/.local/share/flatpak/exports/bin/org.chromium.Chromium"
        )));
        assert!(!is_sandboxed(Path::new("/usr/bin/chromium")));
    }

    #[test]
    fn windows_install_paths_derive_the_user_install_from_home() {
        let paths = install_paths(Path::new("/h"), Os::Windows);
        let local = PathBuf::from("/h").join("AppData").join("Local");
        assert!(paths.contains(&(
            BrowserKind::Opera,
            local.join("Programs").join("Opera").join("opera.exe")
        )));
        assert!(paths.contains(&(
            BrowserKind::Chrome,
            PathBuf::from(r"C:\Program Files")
                .join("Google")
                .join("Chrome")
                .join("Application")
                .join("chrome.exe")
        )));
    }

    #[test]
    fn linux_launch_uses_basic_password_store() {
        let f = flags(BrowserMode::Managed, Os::Linux);
        assert!(f.contains(&"--password-store=basic".to_string()));
        assert!(flags(BrowserMode::Private, Os::Linux).contains(&"--password-store=basic".into()));
        for os in [Os::Mac, Os::Windows] {
            assert!(!flags(BrowserMode::Managed, os)
                .iter()
                .any(|f| f.starts_with("--password-store")));
        }
    }

    #[test]
    fn agent_tabs_are_not_throttled() {
        for mode in [BrowserMode::Managed, BrowserMode::Private] {
            for os in [Os::Mac, Os::Windows, Os::Linux] {
                let f = flags(mode, os);
                for flag in [
                    "--disable-background-timer-throttling",
                    "--disable-renderer-backgrounding",
                    "--disable-backgrounding-occluded-windows",
                ] {
                    assert!(f.contains(&flag.to_string()), "{mode:?} {os:?} {flag}");
                }
            }
        }
    }

    #[test]
    fn attach_mode_launches_nothing_so_it_has_no_flags() {
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            assert!(flags(BrowserMode::Attach, os).is_empty());
        }
    }

    #[test]
    fn started_browsers_get_the_unthrottling_flags() {
        let args = arguments(Path::new("/data/p"), false, None);
        assert!(args.contains(&"--disable-renderer-backgrounding".to_string()));
        assert_eq!(args.last().unwrap(), "about:blank");
    }

    #[test]
    fn choice_decides_which_browser_starts() {
        assert_eq!(
            wanted(&BrowserChoice::Auto)[..2],
            [BrowserKind::Chrome, BrowserKind::Edge]
        );
        assert_eq!(wanted(&BrowserChoice::Auto).len(), 6);
        assert_eq!(
            wanted(&BrowserChoice::Channel {
                channel: "brave".into()
            }),
            vec![BrowserKind::Brave]
        );
        assert_eq!(
            wanted(&BrowserChoice::Channel {
                channel: "msedge".into()
            }),
            vec![BrowserKind::Edge]
        );
    }

    #[test]
    fn a_browser_that_exits_at_once_is_reported_promptly() {
        let profile = std::env::temp_dir().join(format!("gespan-exit-{}", uuid::Uuid::new_v4()));
        let started = Instant::now();
        let error = wait_for_endpoint(&profile, START_TIMEOUT, || {
            Started::Exited("exit status: 1".into())
        })
        .unwrap_err();
        assert!(error.contains("exited"), "{error}");
        assert!(started.elapsed() < HANDOVER_GRACE + Duration::from_secs(2));
        let error = wait_for_endpoint(&profile, START_TIMEOUT, || Started::Gone).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
    }

    /// A private profile a crash left behind: the browser still running on
    /// it is stopped before the folder goes; a process on another profile
    /// with a recorded id is left alone.
    #[cfg(unix)]
    #[test]
    fn startup_stops_browsers_left_on_private_profiles() {
        let app = std::env::temp_dir().join(format!("gespan-orphan-{}", uuid::Uuid::new_v4()));
        let kept = profile_dir(&app);
        let orphan_profile = private_dir(&kept);
        let other_profile = private_dir(&kept);
        std::fs::create_dir_all(&orphan_profile).unwrap();
        std::fs::create_dir_all(&other_profile).unwrap();
        // `sh -c` with a second command keeps its own argv, as Chrome does.
        let spawn = |profile: &Path| {
            Command::new("sh")
                .args([
                    "-c",
                    "sleep 30; :",
                    "browser",
                    &format!("--user-data-dir={}", profile.display()),
                ])
                .spawn()
                .unwrap()
        };
        let mut orphan = spawn(&orphan_profile);
        std::fs::write(orphan_profile.join(PID_FILE), orphan.id().to_string()).unwrap();
        let mut stranger = spawn(&app.join("elsewhere"));
        std::fs::write(other_profile.join(PID_FILE), stranger.id().to_string()).unwrap();
        // The test process is the parent: reap so the exit becomes visible.
        let reaper = std::thread::spawn(move || orphan.wait().unwrap());
        clear_private(&kept);
        assert!(!reaper.join().unwrap().success());
        assert!(
            stranger.try_wait().unwrap().is_none(),
            "an unrelated process was stopped"
        );
        assert!(!private_root(&kept).exists());
        let _ = stranger.kill();
        let _ = stranger.wait();
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn arguments_use_the_own_profile_and_a_free_port() {
        let profile = Path::new("/data/browser-profile");
        let args = arguments(profile, true, Some(1.5));
        assert_eq!(args[0], "--user-data-dir=/data/browser-profile");
        assert!(args.contains(&"--remote-debugging-port=0".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("--headless")));
        assert!(!args.iter().any(|a| a == "--enable-automation"));
        assert!(arguments(profile, false, None).contains(&"--headless=new".to_string()));
        assert!(arguments(profile, false, None).contains(&"--hide-scrollbars".to_string()));
        assert!(!args.iter().any(|a| a == "--hide-scrollbars"));
        // A window has the screen's ratio; a headless browser gets Gespan's.
        assert!(!args
            .iter()
            .any(|a| a.starts_with("--force-device-scale-factor")));
        assert!(arguments(profile, false, Some(1.25))
            .contains(&"--force-device-scale-factor=1.25".to_string()));
        assert!(!arguments(profile, false, Some(1.0))
            .iter()
            .any(|a| a.starts_with("--force-device-scale-factor")));
        assert_eq!(profile_dir(Path::new("/data")), profile);
    }
}
