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
    log_dir_for(
        channel,
        std::env::consts::OS,
        |name| std::env::var_os(name),
        crate::scm::shell::dirs_home().map(PathBuf::from),
        &std::env::temp_dir(),
    )
}

pub fn crash_dir(log_dir: &Path) -> PathBuf {
    log_dir.join("crashes")
}

/// With no home folder (a stripped environment), logs go under `temp`, never
/// relative to the working directory.
pub(crate) fn log_dir_for(
    channel: Channel,
    os: &str,
    env: impl Fn(&str) -> Option<OsString>,
    home: Option<PathBuf>,
    temp: &Path,
) -> PathBuf {
    let var = |name: &str| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let home = home.unwrap_or_else(|| temp.to_path_buf());
    let dev = channel == Channel::Dev;
    match os {
        "macos" => home.join("Library/Logs").join(if dev {
            "dev.gespan.native.dev"
        } else {
            "dev.gespan.native"
        }),
        "windows" => var("LOCALAPPDATA")
            .unwrap_or_else(|| home.join("AppData").join("Local"))
            .join(if dev {
                "dev.gespan.native.dev"
            } else {
                "dev.gespan.native"
            })
            .join("logs"),
        _ => var("XDG_STATE_HOME")
            .unwrap_or_else(|| home.join(".local/state"))
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

    fn home(vars: &[(&str, &str)]) -> Option<PathBuf> {
        vars.iter()
            .find(|(key, value)| *key == "HOME" && !value.is_empty())
            .map(|(_, value)| PathBuf::from(value))
    }

    #[test]
    fn macos_uses_library_logs_with_the_bundle_ids() {
        let vars = [("HOME", "/Users/ada")];
        assert_eq!(
            log_dir_for(
                Channel::Release,
                "macos",
                env(&vars),
                home(&vars),
                Path::new("/tmp")
            ),
            PathBuf::from("/Users/ada/Library/Logs/dev.gespan.native")
        );
        assert_eq!(
            log_dir_for(
                Channel::Dev,
                "macos",
                env(&vars),
                home(&vars),
                Path::new("/tmp")
            ),
            PathBuf::from("/Users/ada/Library/Logs/dev.gespan.native.dev")
        );
    }

    #[test]
    fn linux_prefers_xdg_state_home_and_falls_back_to_local_state() {
        let xdg = [("HOME", "/home/ada"), ("XDG_STATE_HOME", "/state")];
        assert_eq!(
            log_dir_for(
                Channel::Release,
                "linux",
                env(&xdg),
                home(&xdg),
                Path::new("/tmp")
            ),
            PathBuf::from("/state/gespan-native/logs")
        );
        assert_eq!(
            log_dir_for(
                Channel::Dev,
                "linux",
                env(&xdg),
                home(&xdg),
                Path::new("/tmp")
            ),
            PathBuf::from("/state/gespan-native-dev/logs")
        );
        let local = [("HOME", "/home/ada")];
        assert_eq!(
            log_dir_for(
                Channel::Release,
                "linux",
                env(&local),
                home(&local),
                Path::new("/tmp")
            ),
            PathBuf::from("/home/ada/.local/state/gespan-native/logs")
        );
        // An empty XDG_STATE_HOME is unset, per the XDG spec.
        let empty = [("HOME", "/home/ada"), ("XDG_STATE_HOME", "")];
        assert_eq!(
            log_dir_for(
                Channel::Dev,
                "linux",
                env(&empty),
                home(&empty),
                Path::new("/tmp")
            ),
            PathBuf::from("/home/ada/.local/state/gespan-native-dev/logs")
        );
    }

    #[test]
    fn windows_uses_local_app_data() {
        let vars = [("LOCALAPPDATA", r"C:\Users\ada\AppData\Local")];
        assert_eq!(
            log_dir_for(
                Channel::Release,
                "windows",
                env(&vars),
                home(&vars),
                Path::new("/tmp")
            ),
            PathBuf::from(r"C:\Users\ada\AppData\Local")
                .join("dev.gespan.native")
                .join("logs")
        );
        assert_eq!(
            log_dir_for(
                Channel::Dev,
                "windows",
                env(&vars),
                home(&vars),
                Path::new("/tmp")
            ),
            PathBuf::from(r"C:\Users\ada\AppData\Local")
                .join("dev.gespan.native.dev")
                .join("logs")
        );
    }

    #[test]
    fn a_missing_home_never_gives_a_relative_folder() {
        let temp = PathBuf::from("/tmp");
        for os in ["macos", "linux", "windows"] {
            for channel in [Channel::Release, Channel::Dev] {
                let dir = log_dir_for(channel, os, env(&[("HOME", "")]), None, &temp);
                assert!(dir.starts_with(&temp), "{os}: {}", dir.display());
            }
        }
        // Windows without LOCALAPPDATA uses the profile's own AppData\Local.
        assert_eq!(
            log_dir_for(
                Channel::Release,
                "windows",
                env(&[]),
                Some(PathBuf::from(r"C:\Users\ada")),
                &temp
            ),
            PathBuf::from(r"C:\Users\ada")
                .join("AppData")
                .join("Local")
                .join("dev.gespan.native")
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
