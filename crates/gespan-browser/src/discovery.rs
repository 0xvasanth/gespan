//! Discovery: where `DevToolsActivePort` lives for each browser
//! channel, reading it, probing the port, version and policy checks.
//!
//! Discovery never launches a browser and never passes
//! `--remote-debugging-port`; the user enables remote debugging in
//! `chrome://inspect/#remote-debugging` (or `edge://inspect`, and the same
//! page in Brave, Vivaldi, Opera and Arc) and the browser writes the file for
//! us.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::protocol::{BrowserChoice, BrowserKind, MIN_BROWSER_MAJOR};

/// TCP probe before the WebSocket upgrade.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(250);
/// How long we wait for the user to answer Chrome's consent dialog.
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(90);
/// The file the browser writes next to its profile when remote debugging is on.
pub const ACTIVE_PORT_FILE: &str = "DevToolsActivePort";

/// Channel ids in discovery order: Chrome and Edge with
/// their test builds, then the other Chromium browsers.
pub const CHANNELS: [&str; 12] = [
    "chrome",
    "msedge",
    "chrome-beta",
    "chrome-dev",
    "chrome-canary",
    "msedge-beta",
    "msedge-dev",
    "msedge-canary",
    "brave",
    "vivaldi",
    "opera",
    "arc",
];

/// The operating system whose browser layout is wanted. Functions take it as
/// data (with [`Os::current`] at the real call sites) so every layout is
/// testable on any host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// Channels beyond [`CHANNELS`] that "Your own browser" also looks in:
/// Chromium itself and the Flatpak and Snap packages. They have no entry in
/// the settings picker; they are found by Automatic and by name.
pub const EXTRA_CHANNELS: [&str; 4] = [
    "chromium",
    "chromium-snap",
    "chrome-flatpak",
    "chromium-flatpak",
];

/// A browser profile directory discovery may look at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// `chrome`, `msedge`, ... or `custom` for a user-supplied directory.
    pub channel: String,
    pub kind: BrowserKind,
    pub user_data_dir: PathBuf,
}

pub fn channel_kind(channel: &str) -> BrowserKind {
    if channel.starts_with("msedge") {
        BrowserKind::Edge
    } else if channel.starts_with("chrome") {
        BrowserKind::Chrome
    } else if channel.starts_with("brave") {
        BrowserKind::Brave
    } else if channel.starts_with("vivaldi") {
        BrowserKind::Vivaldi
    } else if channel.starts_with("opera") {
        BrowserKind::Opera
    } else if channel.starts_with("arc") {
        BrowserKind::Arc
    } else {
        BrowserKind::Chromium
    }
}

/// The browser behind a profile folder the user picked, from its path.
fn path_kind(path: &str) -> BrowserKind {
    let lower = path.to_ascii_lowercase().replace('\\', "/");
    if lower.contains("edge") {
        BrowserKind::Edge
    } else if lower.contains("brave") {
        BrowserKind::Brave
    } else if lower.contains("vivaldi") {
        BrowserKind::Vivaldi
    } else if lower.contains("opera") {
        BrowserKind::Opera
    } else if lower.contains("/arc/") {
        BrowserKind::Arc
    } else if lower.contains("chrome") {
        BrowserKind::Chrome
    } else {
        BrowserKind::Chromium
    }
}

/// The user-data-dir of a channel under `base` (`%LOCALAPPDATA%` on Windows,
/// the home directory elsewhere).
pub fn user_data_dir_in(channel: &str, base: &Path, os: Os) -> Option<PathBuf> {
    let path = match (os, channel) {
        (Os::Windows, "chrome") => base.join("Google").join("Chrome").join("User Data"),
        (Os::Windows, "chrome-beta") => base.join("Google").join("Chrome Beta").join("User Data"),
        (Os::Windows, "chrome-dev") => base.join("Google").join("Chrome Dev").join("User Data"),
        (Os::Windows, "chrome-canary") => base.join("Google").join("Chrome SxS").join("User Data"),
        (Os::Windows, "msedge") => base.join("Microsoft").join("Edge").join("User Data"),
        (Os::Windows, "msedge-beta") => base.join("Microsoft").join("Edge Beta").join("User Data"),
        (Os::Windows, "msedge-dev") => base.join("Microsoft").join("Edge Dev").join("User Data"),
        (Os::Windows, "msedge-canary") => base.join("Microsoft").join("Edge SxS").join("User Data"),
        (Os::Windows, "brave") => base
            .join("BraveSoftware")
            .join("Brave-Browser")
            .join("User Data"),
        (Os::Windows, "vivaldi") => base.join("Vivaldi").join("User Data"),
        (Os::Windows, "chromium") => base.join("Chromium").join("User Data"),
        // Opera keeps its profile under Roaming, next to `base` (Local).
        (Os::Windows, "opera") => base
            .parent()?
            .join("Roaming")
            .join("Opera Software")
            .join("Opera Stable"),
        (Os::Mac, channel) => {
            let support = base.join("Library").join("Application Support");
            match channel {
                "chrome" => support.join("Google").join("Chrome"),
                "chrome-beta" => support.join("Google").join("Chrome Beta"),
                "chrome-dev" => support.join("Google").join("Chrome Dev"),
                "chrome-canary" => support.join("Google").join("Chrome Canary"),
                "msedge" => support.join("Microsoft Edge"),
                "msedge-beta" => support.join("Microsoft Edge Beta"),
                "msedge-dev" => support.join("Microsoft Edge Dev"),
                "msedge-canary" => support.join("Microsoft Edge Canary"),
                "brave" => support.join("BraveSoftware").join("Brave-Browser"),
                "vivaldi" => support.join("Vivaldi"),
                "chromium" => support.join("Chromium"),
                "opera" => support.join("com.operasoftware.Opera"),
                "arc" => support.join("Arc").join("User Data"),
                _ => return None,
            }
        }
        (Os::Linux, channel) => {
            let config = base.join(".config");
            match channel {
                "chrome" => config.join("google-chrome"),
                "chrome-beta" => config.join("google-chrome-beta"),
                "chrome-dev" => config.join("google-chrome-unstable"),
                "chrome-canary" => config.join("google-chrome-canary"),
                "msedge" => config.join("microsoft-edge"),
                "msedge-beta" => config.join("microsoft-edge-beta"),
                "msedge-dev" => config.join("microsoft-edge-dev"),
                "msedge-canary" => config.join("microsoft-edge-canary"),
                "brave" => config.join("BraveSoftware").join("Brave-Browser"),
                "vivaldi" => config.join("vivaldi"),
                "opera" => config.join("opera"),
                "chromium" => config.join("chromium"),
                // Snap keeps the profile in the package's shared folder.
                "chromium-snap" => base
                    .join("snap")
                    .join("chromium")
                    .join("common")
                    .join("chromium"),
                // Flatpak sandboxes the profile under ~/.var/app/<app id>/config.
                "chrome-flatpak" => base
                    .join(".var/app/com.google.Chrome/config")
                    .join("google-chrome"),
                "chromium-flatpak" => base
                    .join(".var/app/org.chromium.Chromium/config")
                    .join("chromium"),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(path)
}

/// Every profile folder a supported browser may use on `os` for the user
/// whose home folder is `home`, with the browser it belongs to (existing or
/// not). On Windows `%LOCALAPPDATA%` is taken as `home\AppData\Local`.
pub fn profile_dirs(home: &Path, os: Os) -> Vec<(BrowserKind, PathBuf)> {
    let base = match os {
        Os::Windows => home.join("AppData").join("Local"),
        Os::Mac | Os::Linux => home.to_path_buf(),
    };
    CHANNELS
        .iter()
        .chain(EXTRA_CHANNELS.iter())
        .filter_map(|channel| {
            user_data_dir_in(channel, &base, os).map(|dir| (channel_kind(channel), dir))
        })
        .collect()
}

fn base_dir(os: Os) -> Option<PathBuf> {
    let var = match os {
        Os::Windows => "LOCALAPPDATA",
        Os::Mac | Os::Linux => "HOME",
    };
    std::env::var_os(var).map(PathBuf::from)
}

/// Candidates for `choice`, existing directories only.
pub fn candidates(choice: &BrowserChoice) -> Vec<Candidate> {
    let os = Os::current();
    let base = base_dir(os);
    candidates_in(choice, base.as_deref(), os, |path| path.is_dir())
}

pub fn candidates_in(
    choice: &BrowserChoice,
    base: Option<&Path>,
    os: Os,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Candidate> {
    match choice {
        BrowserChoice::UserDataDir { path } => {
            let dir = PathBuf::from(path);
            if !exists(&dir) {
                return Vec::new();
            }
            vec![Candidate {
                channel: "custom".into(),
                kind: path_kind(path),
                user_data_dir: dir,
            }]
        }
        BrowserChoice::Channel { channel } => {
            let Some(base) = base else { return Vec::new() };
            user_data_dir_in(channel, base, os)
                .filter(|dir| exists(dir))
                .map(|dir| Candidate {
                    channel: channel.clone(),
                    kind: channel_kind(channel),
                    user_data_dir: dir,
                })
                .into_iter()
                .collect()
        }
        BrowserChoice::Auto => {
            let Some(base) = base else { return Vec::new() };
            CHANNELS
                .iter()
                .chain(EXTRA_CHANNELS.iter())
                .filter_map(|channel| {
                    let dir = user_data_dir_in(channel, base, os)?;
                    exists(&dir).then(|| Candidate {
                        channel: channel.to_string(),
                        kind: channel_kind(channel),
                        user_data_dir: dir,
                    })
                })
                .collect()
        }
    }
}

/// Contents of `DevToolsActivePort`: the port and the browser endpoint path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivePort {
    pub port: u16,
    /// `/devtools/browser/<id>`; `/devtools/browser` when the second line is missing.
    pub path: String,
}

pub fn parse_active_port(text: &str) -> Option<ActivePort> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let port: u16 = lines.next()?.parse().ok()?;
    if port == 0 {
        return None;
    }
    let path = match lines.next() {
        Some(path) if path.starts_with("/devtools/browser") => path.to_string(),
        Some(_) | None => "/devtools/browser".to_string(),
    };
    Some(ActivePort { port, path })
}

/// What reading `DevToolsActivePort` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortFile {
    /// No file (or one that does not parse): remote debugging is off.
    Missing,
    /// The operating system refused the read (macOS app-data protection
    /// answers EPERM for another app's folder). Whether remote debugging is
    /// on is unknown, so this is never reported as "turn it on".
    Unreadable,
    Present(ActivePort),
}

pub fn read_port_file(user_data_dir: &Path) -> PortFile {
    match std::fs::read_to_string(user_data_dir.join(ACTIVE_PORT_FILE)) {
        Ok(text) => parse_active_port(&text).map_or(PortFile::Missing, PortFile::Present),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => PortFile::Unreadable,
        Err(_) => PortFile::Missing,
    }
}

pub fn read_active_port(user_data_dir: &Path) -> Option<ActivePort> {
    match read_port_file(user_data_dir) {
        PortFile::Present(active) => Some(active),
        PortFile::Missing | PortFile::Unreadable => None,
    }
}

/// `ws://127.0.0.1:<port><path>`: loopback literal so the `Host` header is
/// one the browser accepts.
pub fn ws_url(active: &ActivePort) -> String {
    format!("ws://127.0.0.1:{}{}", active.port, active.path)
}

/// Does anything accept TCP on the port?
pub fn port_accepts(port: u16, timeout: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(stream) => {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            true
        }
        Err(_) => false,
    }
}

/// Outcome of [`locate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Located {
    /// A live endpoint; `ws_url` is what the link should open.
    Found {
        candidate: Candidate,
        active: ActivePort,
        ws_url: String,
    },
    /// The file exists but the port does not answer: browser closed or a
    /// stale file after a crash.
    StalePort { candidate: Candidate, port: u16 },
    /// No readable file, and the system refused to let Gespan read this
    /// profile's: remote debugging may well be on.
    Unreadable { candidate: Candidate },
    /// Profiles exist but none has the file: remote debugging is off.
    NeedsToggle { candidates: Vec<Candidate> },
    /// No supported browser profile on this machine (or the chosen one is missing).
    NoBrowser,
}

/// Candidates, port file, TCP probe. Pure file and TCP work; the WebSocket
/// is the link's job.
pub fn locate(choice: &BrowserChoice) -> Located {
    locate_with(candidates(choice), read_port_file, |port| {
        port_accepts(port, PROBE_TIMEOUT)
    })
}

pub fn locate_with(
    candidates: Vec<Candidate>,
    read: impl Fn(&Path) -> PortFile,
    probe: impl Fn(u16) -> bool,
) -> Located {
    if candidates.is_empty() {
        return Located::NoBrowser;
    }
    let mut stale: Option<(Candidate, u16)> = None;
    let mut unreadable: Option<Candidate> = None;
    for candidate in &candidates {
        let active = match read(&candidate.user_data_dir) {
            PortFile::Present(active) => active,
            PortFile::Unreadable => {
                unreadable.get_or_insert_with(|| candidate.clone());
                continue;
            }
            PortFile::Missing => continue,
        };
        if probe(active.port) {
            let ws_url = ws_url(&active);
            return Located::Found {
                candidate: candidate.clone(),
                active,
                ws_url,
            };
        }
        if stale.is_none() {
            stale = Some((candidate.clone(), active.port));
        }
    }
    match (stale, unreadable) {
        (Some((candidate, port)), _) => Located::StalePort { candidate, port },
        (None, Some(candidate)) => Located::Unreadable { candidate },
        (None, None) => Located::NeedsToggle { candidates },
    }
}

/// The instruction shown in `NeedsToggle` (UI and agent alike).
pub fn setup_instruction(kind: Option<BrowserKind>) -> String {
    const AFTER: &str =
        "turn on remote debugging, then retry. Retrying without that step will not help.";
    match kind {
        Some(kind) => format!("Open {} in {} and {AFTER}", inspect_page(kind), kind.label()),
        None => format!("Open chrome://inspect/#remote-debugging (Chrome, Brave, Vivaldi, Opera, Arc) or edge://inspect (Edge), {AFTER}"),
    }
}

/// Where a browser turns remote debugging on.
pub fn inspect_page(kind: BrowserKind) -> &'static str {
    match kind {
        BrowserKind::Edge => "edge://inspect",
        BrowserKind::Brave => "brave://inspect/#remote-debugging",
        _ => "chrome://inspect/#remote-debugging",
    }
}

/// Browser name and major version from `Browser.getVersion.product`
/// (`Chrome/144.0.7559.60`, `HeadlessChrome/144.0.0.0`, `Edg/144.0.0.0`).
pub fn parse_version(product: &str) -> Option<(String, u32)> {
    let (name, version) = product.trim().split_once('/')?;
    let major: u32 = version.split('.').next()?.trim().parse().ok()?;
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), major))
}

pub fn version_supported(major: u32) -> bool {
    major >= MIN_BROWSER_MAJOR
}

/// Best-effort check whether an administrator policy turns remote debugging
/// off (`RemoteDebuggingAllowed = false`). Only used for the message in
/// `NeedsToggle`; nothing is ever bypassed.
pub fn policy_blocks_debugging(kind: BrowserKind) -> Option<String> {
    match Os::current() {
        Os::Linux => {
            let dir = match kind {
                BrowserKind::Edge => "/etc/opt/edge/policies/managed",
                BrowserKind::Chrome => "/etc/opt/chrome/policies/managed",
                BrowserKind::Chromium => "/etc/chromium/policies/managed",
                BrowserKind::Brave => "/etc/brave/policies/managed",
                BrowserKind::Vivaldi | BrowserKind::Opera | BrowserKind::Arc => return None,
            };
            linux_policy_blocks(Path::new(dir))
        }
        Os::Mac => {
            let domain = match kind {
                BrowserKind::Edge => "com.microsoft.Edge",
                BrowserKind::Chrome | BrowserKind::Chromium => "com.google.Chrome",
                BrowserKind::Brave => "com.brave.Browser",
                BrowserKind::Vivaldi | BrowserKind::Opera | BrowserKind::Arc => return None,
            };
            let output = std::process::Command::new("defaults")
                .args([
                    "read",
                    &format!("/Library/Managed Preferences/{domain}"),
                    "RemoteDebuggingAllowed",
                ])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            (output.status.success() && text.trim() == "0")
                .then(|| format!("Managed preference {domain} RemoteDebuggingAllowed = 0"))
        }
        Os::Windows => {
            let key = match kind {
                BrowserKind::Edge => r"Software\Policies\Microsoft\Edge",
                BrowserKind::Chrome | BrowserKind::Chromium => r"Software\Policies\Google\Chrome",
                BrowserKind::Brave => r"Software\Policies\BraveSoftware\Brave",
                BrowserKind::Vivaldi | BrowserKind::Opera | BrowserKind::Arc => return None,
            };
            for hive in ["HKLM", "HKCU"] {
                let mut command = std::process::Command::new("reg");
                command.args([
                    "query",
                    &format!(r"{hive}\{key}"),
                    "/v",
                    "RemoteDebuggingAllowed",
                ]);
                crate::util::hide_window_console(&mut command);
                let output = command.output().ok()?;
                let text = String::from_utf8_lossy(&output.stdout);
                if output.status.success() && text.contains("0x0") {
                    return Some(format!(r"{hive}\{key}\RemoteDebuggingAllowed = 0"));
                }
            }
            None
        }
    }
}

fn linux_policy_blocks(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value.get("RemoteDebuggingAllowed") == Some(&serde_json::Value::Bool(false)) {
            return Some(format!(
                "{} sets RemoteDebuggingAllowed = false",
                path.display()
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn user_data_dirs_follow_the_spec_table() {
        let win = Path::new(r"C:\Users\o\AppData\Local");
        assert_eq!(
            user_data_dir_in("chrome", win, Os::Windows).unwrap(),
            win.join("Google").join("Chrome").join("User Data")
        );
        assert_eq!(
            user_data_dir_in("msedge-canary", win, Os::Windows).unwrap(),
            win.join("Microsoft").join("Edge SxS").join("User Data")
        );
        let home = Path::new("/Users/o");
        assert_eq!(
            user_data_dir_in("chrome-canary", home, Os::Mac).unwrap(),
            PathBuf::from("/Users/o/Library/Application Support/Google/Chrome Canary")
        );
        assert_eq!(
            user_data_dir_in("msedge", home, Os::Mac).unwrap(),
            PathBuf::from("/Users/o/Library/Application Support/Microsoft Edge")
        );
        let home = Path::new("/home/o");
        assert_eq!(
            user_data_dir_in("chrome-dev", home, Os::Linux).unwrap(),
            PathBuf::from("/home/o/.config/google-chrome-unstable")
        );
        assert_eq!(
            user_data_dir_in("msedge-beta", home, Os::Linux).unwrap(),
            PathBuf::from("/home/o/.config/microsoft-edge-beta")
        );
        assert!(user_data_dir_in("firefox", home, Os::Linux).is_none());
        assert_eq!(channel_kind("msedge-dev"), BrowserKind::Edge);
        assert_eq!(channel_kind("chrome"), BrowserKind::Chrome);
    }

    #[test]
    fn other_chromium_browsers_have_their_profile_folders() {
        let win = Path::new(r"C:\Users\o\AppData\Local");
        assert_eq!(
            user_data_dir_in("brave", win, Os::Windows).unwrap(),
            win.join("BraveSoftware")
                .join("Brave-Browser")
                .join("User Data")
        );
        // Forward slashes, so the parent folder is found on every test host.
        let local = Path::new("C:/Users/o/AppData/Local");
        assert_eq!(
            user_data_dir_in("opera", local, Os::Windows).unwrap(),
            Path::new("C:/Users/o/AppData")
                .join("Roaming")
                .join("Opera Software")
                .join("Opera Stable")
        );
        assert!(user_data_dir_in("arc", win, Os::Windows).is_none());
        let mac = Path::new("/Users/o");
        assert_eq!(
            user_data_dir_in("arc", mac, Os::Mac).unwrap(),
            PathBuf::from("/Users/o/Library/Application Support/Arc/User Data")
        );
        assert_eq!(
            user_data_dir_in("vivaldi", mac, Os::Mac).unwrap(),
            PathBuf::from("/Users/o/Library/Application Support/Vivaldi")
        );
        let linux = Path::new("/home/o");
        assert_eq!(
            user_data_dir_in("brave", linux, Os::Linux).unwrap(),
            PathBuf::from("/home/o/.config/BraveSoftware/Brave-Browser")
        );
        assert_eq!(
            user_data_dir_in("opera", linux, Os::Linux).unwrap(),
            PathBuf::from("/home/o/.config/opera")
        );
        assert!(user_data_dir_in("arc", linux, Os::Linux).is_none());
        for (channel, kind) in [
            ("brave", BrowserKind::Brave),
            ("vivaldi", BrowserKind::Vivaldi),
            ("opera", BrowserKind::Opera),
            ("arc", BrowserKind::Arc),
        ] {
            assert_eq!(channel_kind(channel), kind);
            assert!(CHANNELS.contains(&channel));
        }
        assert_eq!(
            path_kind(r"C:\Users\o\AppData\Local\BraveSoftware\Brave-Browser\User Data"),
            BrowserKind::Brave
        );
        assert_eq!(
            path_kind("/Users/o/Library/Application Support/Arc/User Data"),
            BrowserKind::Arc
        );
        assert_eq!(path_kind("/snap/chromium/current"), BrowserKind::Chromium);
        assert!(setup_instruction(Some(BrowserKind::Brave)).contains("brave://inspect"));
        assert!(setup_instruction(Some(BrowserKind::Opera)).contains("in Opera"));
    }

    #[test]
    fn candidates_respect_the_choice_and_skip_missing_dirs() {
        let home = Path::new("/home/o");
        let existing = |p: &Path| {
            p == Path::new("/home/o/.config/microsoft-edge")
                || p == Path::new("/home/o/.config/google-chrome-beta")
                || p == Path::new("/snap/chromium/current")
        };
        let auto = candidates_in(&BrowserChoice::Auto, Some(home), Os::Linux, existing);
        let channels: Vec<&str> = auto.iter().map(|c| c.channel.as_str()).collect();
        assert_eq!(channels, vec!["msedge", "chrome-beta"]);
        let one = candidates_in(
            &BrowserChoice::Channel {
                channel: "chrome".into(),
            },
            Some(home),
            Os::Linux,
            existing,
        );
        assert!(one.is_empty());
        let custom = candidates_in(
            &BrowserChoice::UserDataDir {
                path: "/snap/chromium/current".into(),
            },
            Some(home),
            Os::Linux,
            existing,
        );
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].channel, "custom");
        assert_eq!(custom[0].kind, BrowserKind::Chromium);
        assert!(candidates_in(&BrowserChoice::Auto, None, Os::Linux, existing).is_empty());
    }

    #[test]
    fn linux_finds_chromium_flatpak_and_snap() {
        let home = Path::new("/home/o");
        let dirs = profile_dirs(home, Os::Linux);
        for (kind, dir) in [
            (BrowserKind::Chromium, "/home/o/.config/chromium"),
            (
                BrowserKind::Chrome,
                "/home/o/.var/app/com.google.Chrome/config/google-chrome",
            ),
            (
                BrowserKind::Chromium,
                "/home/o/.var/app/org.chromium.Chromium/config/chromium",
            ),
            (
                BrowserKind::Chromium,
                "/home/o/snap/chromium/common/chromium",
            ),
        ] {
            assert!(
                dirs.contains(&(kind, PathBuf::from(dir))),
                "missing {kind:?} {dir}"
            );
        }
        // The browsers found before stay found.
        assert!(dirs.contains(&(
            BrowserKind::Chrome,
            PathBuf::from("/home/o/.config/google-chrome")
        )));
        assert!(dirs.contains(&(
            BrowserKind::Brave,
            PathBuf::from("/home/o/.config/BraveSoftware/Brave-Browser")
        )));
    }

    #[test]
    fn profile_dirs_follow_each_os_layout() {
        let mac = profile_dirs(Path::new("/Users/o"), Os::Mac);
        let support = PathBuf::from("/Users/o/Library/Application Support");
        assert!(mac.contains(&(BrowserKind::Arc, support.join("Arc").join("User Data"))));
        assert!(mac.contains(&(BrowserKind::Chromium, support.join("Chromium"))));
        assert!(!mac
            .iter()
            .any(|(_, dir)| dir.to_string_lossy().contains(".config")));

        // Windows derives %LOCALAPPDATA% from the home folder; Opera lives
        // under Roaming next to it. Arc and the Linux-only packages are absent.
        let win = profile_dirs(Path::new("/h"), Os::Windows);
        let local = PathBuf::from("/h").join("AppData").join("Local");
        assert!(win.contains(&(
            BrowserKind::Chrome,
            local.join("Google").join("Chrome").join("User Data")
        )));
        assert!(win.contains(&(
            BrowserKind::Opera,
            PathBuf::from("/h")
                .join("AppData")
                .join("Roaming")
                .join("Opera Software")
                .join("Opera Stable")
        )));
        assert!(!win.iter().any(|(kind, _)| *kind == BrowserKind::Arc));
        assert!(!win
            .iter()
            .any(|(_, dir)| dir.to_string_lossy().contains("flatpak")
                || dir.to_string_lossy().contains(".var")));
        let linux = profile_dirs(Path::new("/home/o"), Os::Linux);
        assert!(!linux.iter().any(|(kind, _)| *kind == BrowserKind::Arc));
    }

    #[test]
    fn auto_discovery_looks_in_flatpak_and_snap_profiles() {
        let home = Path::new("/home/o");
        let snap = Path::new("/home/o/snap/chromium/common/chromium");
        let found = candidates_in(&BrowserChoice::Auto, Some(home), Os::Linux, |p| p == snap);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, BrowserKind::Chromium);
        assert_eq!(found[0].user_data_dir, snap);
    }

    #[test]
    fn active_port_file_parses_both_shapes() {
        let active = parse_active_port("9222\n/devtools/browser/abc-123\n").unwrap();
        assert_eq!(active.port, 9222);
        assert_eq!(active.path, "/devtools/browser/abc-123");
        assert_eq!(
            ws_url(&active),
            "ws://127.0.0.1:9222/devtools/browser/abc-123"
        );
        let bare = parse_active_port("1234").unwrap();
        assert_eq!(bare.path, "/devtools/browser");
        assert!(parse_active_port("").is_none());
        assert!(parse_active_port("0\n/devtools/browser/x").is_none());
        assert!(parse_active_port("abc\n/devtools/browser/x").is_none());
        let odd = parse_active_port("4000\n/not/devtools").unwrap();
        assert_eq!(odd.path, "/devtools/browser");
    }

    #[test]
    fn locate_classifies_missing_stale_and_live_endpoints() {
        let candidate = |channel: &str| Candidate {
            channel: channel.into(),
            kind: channel_kind(channel),
            user_data_dir: PathBuf::from(format!("/profiles/{channel}")),
        };
        assert_eq!(
            locate_with(Vec::new(), |_| PortFile::Missing, |_| true),
            Located::NoBrowser
        );
        let needs = locate_with(
            vec![candidate("chrome"), candidate("msedge")],
            |_| PortFile::Missing,
            |_| true,
        );
        assert!(matches!(needs, Located::NeedsToggle { candidates } if candidates.len() == 2));
        let stale = locate_with(
            vec![candidate("chrome")],
            |_| {
                PortFile::Present(ActivePort {
                    port: 1,
                    path: "/devtools/browser/x".into(),
                })
            },
            |_| false,
        );
        assert_eq!(
            stale,
            Located::StalePort {
                candidate: candidate("chrome"),
                port: 1
            }
        );
        // A stale first candidate does not hide a live second one.
        let found = locate_with(
            vec![candidate("chrome"), candidate("msedge")],
            |dir| {
                let port = if dir.ends_with("chrome") { 1 } else { 2 };
                PortFile::Present(ActivePort {
                    port,
                    path: format!("/devtools/browser/{port}"),
                })
            },
            |port| port == 2,
        );
        assert!(
            matches!(found, Located::Found { candidate, ws_url, .. } if candidate.channel == "msedge" && ws_url == "ws://127.0.0.1:2/devtools/browser/2")
        );
    }

    #[test]
    fn locate_reads_real_files_and_probes_real_ports() {
        let dir = std::env::temp_dir().join(format!("gespan-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let candidate = Candidate {
            channel: "chrome".into(),
            kind: BrowserKind::Chrome,
            user_data_dir: dir.clone(),
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(
            dir.join(ACTIVE_PORT_FILE),
            format!("{port}\n/devtools/browser/live\n"),
        )
        .unwrap();
        let located = locate_with(vec![candidate.clone()], read_port_file, |p| {
            port_accepts(p, PROBE_TIMEOUT)
        });
        assert!(
            matches!(located, Located::Found { ws_url, .. } if ws_url == format!("ws://127.0.0.1:{port}/devtools/browser/live"))
        );
        drop(listener);
        // Port 1 is never listening on loopback.
        std::fs::write(dir.join(ACTIVE_PORT_FILE), "1\n/devtools/browser/dead\n").unwrap();
        let located = locate_with(vec![candidate.clone()], read_port_file, |p| {
            port_accepts(p, PROBE_TIMEOUT)
        });
        assert_eq!(
            located,
            Located::StalePort {
                candidate: candidate.clone(),
                port: 1
            }
        );
        std::fs::remove_file(dir.join(ACTIVE_PORT_FILE)).unwrap();
        let located = locate_with(vec![candidate], read_port_file, |_| true);
        assert!(matches!(located, Located::NeedsToggle { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refused read is "unreadable", never "remote debugging is off"; a
    /// live or stale file elsewhere still wins, and a missing file next to
    /// it does not hide it.
    #[test]
    fn locate_tells_an_unreadable_profile_from_a_missing_file() {
        let candidate = |channel: &str| Candidate {
            channel: channel.into(),
            kind: channel_kind(channel),
            user_data_dir: PathBuf::from(format!("/profiles/{channel}")),
        };
        let both = || vec![candidate("chrome"), candidate("brave")];
        let read = |chrome: PortFile, brave: PortFile| {
            move |dir: &Path| {
                if dir.ends_with("chrome") {
                    chrome.clone()
                } else {
                    brave.clone()
                }
            }
        };
        let live = PortFile::Present(ActivePort {
            port: 2,
            path: "/devtools/browser/2".into(),
        });
        assert_eq!(
            locate_with(
                both(),
                read(PortFile::Unreadable, PortFile::Missing),
                |_| true
            ),
            Located::Unreadable {
                candidate: candidate("chrome")
            }
        );
        assert_eq!(
            locate_with(
                both(),
                read(PortFile::Missing, PortFile::Unreadable),
                |_| true
            ),
            Located::Unreadable {
                candidate: candidate("brave")
            }
        );
        assert!(matches!(
            locate_with(both(), read(PortFile::Unreadable, live.clone()), |_| true),
            Located::Found { candidate, .. } if candidate.channel == "brave"
        ));
        assert!(matches!(
            locate_with(both(), read(PortFile::Unreadable, live), |_| false),
            Located::StalePort { candidate, .. } if candidate.channel == "brave"
        ));
        assert!(matches!(
            locate_with(both(), read(PortFile::Missing, PortFile::Missing), |_| true),
            Located::NeedsToggle { .. }
        ));
    }

    /// The real read: permission denied is told apart from no file and
    /// from a file that does not parse.
    #[cfg(unix)]
    #[test]
    fn a_permission_error_reading_the_port_file_is_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gespan-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_port_file(&dir), PortFile::Missing);
        let file = dir.join(ACTIVE_PORT_FILE);
        std::fs::write(&file, "not a port\n").unwrap();
        assert_eq!(read_port_file(&dir), PortFile::Missing);
        std::fs::write(&file, "4000\n/devtools/browser/x\n").unwrap();
        assert!(matches!(read_port_file(&dir), PortFile::Present(a) if a.port == 4000));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&file).is_err() {
            assert_eq!(read_port_file(&dir), PortFile::Unreadable);
            assert_eq!(read_active_port(&dir), None);
        }
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versions_parse_and_the_floor_is_144() {
        assert_eq!(
            parse_version("Chrome/144.0.7559.60").unwrap(),
            ("Chrome".to_string(), 144)
        );
        assert_eq!(parse_version("HeadlessChrome/150.0.0.0").unwrap().1, 150);
        assert_eq!(
            parse_version(" Edg/143.0.1 ").unwrap(),
            ("Edg".to_string(), 143)
        );
        assert!(parse_version("Chrome").is_none());
        assert!(parse_version("/144.0").is_none());
        assert!(version_supported(144));
        assert!(!version_supported(143));
        assert!(setup_instruction(Some(BrowserKind::Edge)).contains("edge://inspect"));
        assert!(setup_instruction(None).contains("chrome://inspect"));
    }

    #[test]
    fn linux_policy_scan_reads_managed_json() {
        let dir = std::env::temp_dir().join(format!("gespan-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(linux_policy_blocks(&dir).is_none());
        std::fs::write(
            dir.join("other.json"),
            r#"{"RemoteDebuggingAllowed": true}"#,
        )
        .unwrap();
        assert!(linux_policy_blocks(&dir).is_none());
        std::fs::write(
            dir.join("lock.json"),
            r#"{"RemoteDebuggingAllowed": false}"#,
        )
        .unwrap();
        assert!(linux_policy_blocks(&dir).unwrap().contains("lock.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
