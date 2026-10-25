use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Which data the app runs against. Development logs never mix with release
/// logs, the same way `--dev-data` keeps its own database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Release,
    Dev,
}

pub fn log_dir(channel: Channel) -> PathBuf {
    log_dir_for(channel, std::env::consts::OS, |name| std::env::var_os(name))
}

pub fn crash_dir(log_dir: &Path) -> PathBuf {
    log_dir.join("crashes")
}

pub(crate) fn log_dir_for(
    channel: Channel,
    os: &str,
    env: impl Fn(&str) -> Option<OsString>,
) -> PathBuf {
    let var = |name: &str| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let home = || var("HOME").unwrap_or_default();
    let dev = channel == Channel::Dev;
    match os {
        "macos" => home().join("Library/Logs").join(if dev {
            "dev.gespan.native.dev"
        } else {
            "dev.gespan.native"
        }),
        "windows" => var("LOCALAPPDATA")
            .unwrap_or_default()
            .join(if dev {
                "dev.gespan.native.dev"
            } else {
                "dev.gespan.native"
            })
            .join("logs"),
        _ => var("XDG_STATE_HOME")
            .unwrap_or_else(|| home().join(".local/state"))
            .join(if dev {
                "gespan-native-dev"
            } else {
                "gespan-native"
            })
            .join("logs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn macos_uses_library_logs_with_the_bundle_ids() {
        let vars = [("HOME", "/Users/ada")];
        assert_eq!(
            log_dir_for(Channel::Release, "macos", env(&vars)),
            PathBuf::from("/Users/ada/Library/Logs/dev.gespan.native")
        );
        assert_eq!(
            log_dir_for(Channel::Dev, "macos", env(&vars)),
            PathBuf::from("/Users/ada/Library/Logs/dev.gespan.native.dev")
        );
    }

    #[test]
    fn linux_prefers_xdg_state_home_and_falls_back_to_local_state() {
        let xdg = [("HOME", "/home/ada"), ("XDG_STATE_HOME", "/state")];
        assert_eq!(
            log_dir_for(Channel::Release, "linux", env(&xdg)),
            PathBuf::from("/state/gespan-native/logs")
        );
        assert_eq!(
            log_dir_for(Channel::Dev, "linux", env(&xdg)),
            PathBuf::from("/state/gespan-native-dev/logs")
        );
        let home = [("HOME", "/home/ada")];
        assert_eq!(
            log_dir_for(Channel::Release, "linux", env(&home)),
            PathBuf::from("/home/ada/.local/state/gespan-native/logs")
        );
        // An empty XDG_STATE_HOME is unset, per the XDG spec.
        let empty = [("HOME", "/home/ada"), ("XDG_STATE_HOME", "")];
        assert_eq!(
            log_dir_for(Channel::Dev, "linux", env(&empty)),
            PathBuf::from("/home/ada/.local/state/gespan-native-dev/logs")
        );
    }

    #[test]
    fn windows_uses_local_app_data() {
        let vars = [("LOCALAPPDATA", r"C:\Users\ada\AppData\Local")];
        assert_eq!(
            log_dir_for(Channel::Release, "windows", env(&vars)),
            PathBuf::from(r"C:\Users\ada\AppData\Local")
                .join("dev.gespan.native")
                .join("logs")
        );
        assert_eq!(
            log_dir_for(Channel::Dev, "windows", env(&vars)),
            PathBuf::from(r"C:\Users\ada\AppData\Local")
                .join("dev.gespan.native.dev")
                .join("logs")
        );
    }

    #[test]
    fn crashes_sit_next_to_the_logs() {
        assert_eq!(
            crash_dir(Path::new("/logs")),
            PathBuf::from("/logs/crashes")
        );
    }
}
