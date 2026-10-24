//! Opening a project folder in an installed code editor, ported from
//! src-tauri/src/external_editor.rs.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(not(windows))]
use crate::scm::shell::dirs_home;
use crate::scm::shell::{apply_gui_env, expand_home, resolve_gui_binary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExternalEditor {
    pub id: &'static str,
    pub name: &'static str,
}

struct EditorDefinition {
    id: &'static str,
    name: &'static str,
    commands: &'static [&'static str],
    #[cfg(target_os = "macos")]
    mac_apps: &'static [&'static str],
    #[cfg(windows)]
    windows_paths: &'static [(&'static str, &'static str)],
}

const EDITORS: &[EditorDefinition] = &[
    EditorDefinition {
        id: "vscode",
        name: "Visual Studio Code",
        commands: &["code"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Visual Studio Code.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/Microsoft VS Code/Code.exe"),
            ("ProgramFiles", "Microsoft VS Code/Code.exe"),
            ("ProgramFiles(x86)", "Microsoft VS Code/Code.exe"),
        ],
    },
    EditorDefinition {
        id: "vscode-insiders",
        name: "Visual Studio Code Insiders",
        commands: &["code-insiders"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Visual Studio Code - Insiders.app"],
        #[cfg(windows)]
        windows_paths: &[
            (
                "LOCALAPPDATA",
                "Programs/Microsoft VS Code Insiders/Code - Insiders.exe",
            ),
            (
                "ProgramFiles",
                "Microsoft VS Code Insiders/Code - Insiders.exe",
            ),
            (
                "ProgramFiles(x86)",
                "Microsoft VS Code Insiders/Code - Insiders.exe",
            ),
        ],
    },
    EditorDefinition {
        id: "vscodium",
        name: "VSCodium",
        commands: &["codium"],
        #[cfg(target_os = "macos")]
        mac_apps: &["VSCodium.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/VSCodium/VSCodium.exe"),
            ("ProgramFiles", "VSCodium/VSCodium.exe"),
            ("ProgramFiles(x86)", "VSCodium/VSCodium.exe"),
        ],
    },
    EditorDefinition {
        id: "cursor",
        name: "Cursor",
        commands: &["cursor"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Cursor.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/cursor/Cursor.exe"),
            ("ProgramFiles", "Cursor/Cursor.exe"),
            ("ProgramFiles(x86)", "Cursor/Cursor.exe"),
        ],
    },
    EditorDefinition {
        id: "zed",
        name: "Zed",
        commands: &["zed"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Zed.app", "Zed Preview.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/Zed/Zed.exe"),
            ("ProgramFiles", "Zed/Zed.exe"),
        ],
    },
    EditorDefinition {
        id: "windsurf",
        name: "Windsurf",
        commands: &["windsurf"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Windsurf.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/Windsurf/Windsurf.exe"),
            ("ProgramFiles", "Windsurf/Windsurf.exe"),
        ],
    },
    EditorDefinition {
        id: "sublime-text",
        name: "Sublime Text",
        commands: &["subl", "sublime_text"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Sublime Text.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("ProgramFiles", "Sublime Text/sublime_text.exe"),
            ("ProgramFiles(x86)", "Sublime Text/sublime_text.exe"),
        ],
    },
    EditorDefinition {
        id: "intellij-idea",
        name: "IntelliJ IDEA",
        commands: &["idea", "intellij-idea-ultimate", "intellij-idea-community"],
        #[cfg(target_os = "macos")]
        mac_apps: &[
            "IntelliJ IDEA.app",
            "IntelliJ IDEA Ultimate.app",
            "IntelliJ IDEA CE.app",
            "IntelliJ IDEA Community Edition.app",
        ],
        #[cfg(windows)]
        windows_paths: &[
            (
                "LOCALAPPDATA",
                "Programs/IntelliJ IDEA Ultimate/bin/idea64.exe",
            ),
            (
                "LOCALAPPDATA",
                "Programs/IntelliJ IDEA Community Edition/bin/idea64.exe",
            ),
            ("ProgramFiles", "JetBrains/IntelliJ IDEA 2*/bin/idea64.exe"),
            (
                "ProgramFiles",
                "JetBrains/IntelliJ IDEA Community Edition*/bin/idea64.exe",
            ),
        ],
    },
    EditorDefinition {
        id: "webstorm",
        name: "WebStorm",
        commands: &["webstorm"],
        #[cfg(target_os = "macos")]
        mac_apps: &["WebStorm.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/WebStorm/bin/webstorm64.exe"),
            ("ProgramFiles", "JetBrains/WebStorm*/bin/webstorm64.exe"),
        ],
    },
    EditorDefinition {
        id: "pycharm",
        name: "PyCharm",
        commands: &["pycharm", "pycharm-professional", "pycharm-community"],
        #[cfg(target_os = "macos")]
        mac_apps: &[
            "PyCharm.app",
            "PyCharm Professional Edition.app",
            "PyCharm CE.app",
            "PyCharm Community Edition.app",
        ],
        #[cfg(windows)]
        windows_paths: &[
            (
                "LOCALAPPDATA",
                "Programs/PyCharm Professional/bin/pycharm64.exe",
            ),
            ("LOCALAPPDATA", "Programs/PyCharm/bin/pycharm64.exe"),
            (
                "LOCALAPPDATA",
                "Programs/PyCharm Community/bin/pycharm64.exe",
            ),
            ("ProgramFiles", "JetBrains/PyCharm*/bin/pycharm64.exe"),
        ],
    },
    EditorDefinition {
        id: "goland",
        name: "GoLand",
        commands: &["goland"],
        #[cfg(target_os = "macos")]
        mac_apps: &["GoLand.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/GoLand/bin/goland64.exe"),
            ("ProgramFiles", "JetBrains/GoLand*/bin/goland64.exe"),
        ],
    },
    EditorDefinition {
        id: "rustrover",
        name: "RustRover",
        commands: &["rustrover"],
        #[cfg(target_os = "macos")]
        mac_apps: &["RustRover.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/RustRover/bin/rustrover64.exe"),
            ("ProgramFiles", "JetBrains/RustRover*/bin/rustrover64.exe"),
        ],
    },
    EditorDefinition {
        id: "clion",
        name: "CLion",
        commands: &["clion"],
        #[cfg(target_os = "macos")]
        mac_apps: &["CLion.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/CLion/bin/clion64.exe"),
            ("ProgramFiles", "JetBrains/CLion*/bin/clion64.exe"),
        ],
    },
    EditorDefinition {
        id: "phpstorm",
        name: "PhpStorm",
        commands: &["phpstorm"],
        #[cfg(target_os = "macos")]
        mac_apps: &["PhpStorm.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/PhpStorm/bin/phpstorm64.exe"),
            ("ProgramFiles", "JetBrains/PhpStorm*/bin/phpstorm64.exe"),
        ],
    },
    EditorDefinition {
        id: "rider",
        name: "Rider",
        commands: &["rider"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Rider.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/Rider/bin/rider64.exe"),
            ("ProgramFiles", "JetBrains/JetBrains Rider*/bin/rider64.exe"),
        ],
    },
    EditorDefinition {
        id: "rubymine",
        name: "RubyMine",
        commands: &["rubymine"],
        #[cfg(target_os = "macos")]
        mac_apps: &["RubyMine.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/RubyMine/bin/rubymine64.exe"),
            ("ProgramFiles", "JetBrains/RubyMine*/bin/rubymine64.exe"),
        ],
    },
    EditorDefinition {
        id: "android-studio",
        name: "Android Studio",
        commands: &["studio", "android-studio"],
        #[cfg(target_os = "macos")]
        mac_apps: &["Android Studio.app"],
        #[cfg(windows)]
        windows_paths: &[
            ("LOCALAPPDATA", "Programs/Android Studio/bin/studio64.exe"),
            ("ProgramFiles", "Android/Android Studio/bin/studio64.exe"),
        ],
    },
];

enum EditorLauncher {
    Command(PathBuf),
    #[cfg(target_os = "macos")]
    MacApp(PathBuf),
}

fn definition(id: &str) -> Option<&'static EditorDefinition> {
    EDITORS.iter().find(|editor| editor.id == id)
}

#[cfg(target_os = "macos")]
fn installed_mac_app(editor: &EditorDefinition) -> Option<PathBuf> {
    let user_applications = dirs_home().map(|home| PathBuf::from(home).join("Applications"));
    editor.mac_apps.iter().find_map(|name| {
        user_applications
            .as_ref()
            .map(|root| root.join(name))
            .filter(|path| path.is_dir())
            .or_else(|| {
                let path = Path::new("/Applications").join(name);
                path.is_dir().then_some(path)
            })
    })
}

#[cfg(windows)]
fn installed_windows_app(editor: &EditorDefinition) -> Option<PathBuf> {
    editor
        .windows_paths
        .iter()
        .find_map(|(variable, relative)| {
            let root = std::env::var_os(variable)?;
            installed_file(Path::new(&root), relative)
        })
}

/// `relative` under `root`, where a path segment ending in `*` matches
/// versioned install folders such as `JetBrains/WebStorm 2025.2`. The
/// newest-sorting match wins.
#[cfg(any(windows, test))]
fn installed_file(root: &Path, relative: &str) -> Option<PathBuf> {
    let Some((before, after)) = relative.split_once('*') else {
        let path = root.join(relative);
        return path.is_file().then_some(path);
    };
    let (parent, prefix) = before.rsplit_once('/').unwrap_or(("", before));
    let rest = after.strip_prefix('/')?;
    let mut matches: Vec<PathBuf> = std::fs::read_dir(root.join(parent))
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(prefix))
        })
        .map(|entry| entry.path().join(rest))
        .filter(|path| path.is_file())
        .collect();
    matches.sort();
    matches.pop()
}

/// JetBrains Toolbox writes a launcher script for each IDE it installs here,
/// but only shells it has configured have the folder on PATH.
#[cfg(target_os = "macos")]
fn toolbox_scripts_dir() -> Option<PathBuf> {
    dirs_home().map(|home| {
        PathBuf::from(home).join("Library/Application Support/JetBrains/Toolbox/scripts")
    })
}

#[cfg(windows)]
fn toolbox_scripts_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("JetBrains/Toolbox/scripts"))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn toolbox_scripts_dir() -> Option<PathBuf> {
    dirs_home().map(|home| PathBuf::from(home).join(".local/share/JetBrains/Toolbox/scripts"))
}

fn toolbox_script(dir: &Path, editor: &EditorDefinition) -> Option<PathBuf> {
    editor.commands.iter().find_map(|command| {
        let path = if cfg!(windows) {
            dir.join(format!("{command}.cmd"))
        } else {
            dir.join(command)
        };
        path.is_file().then_some(path)
    })
}

fn resolve_editor(editor: &EditorDefinition) -> Option<EditorLauncher> {
    #[cfg(target_os = "macos")]
    if let Some(path) = installed_mac_app(editor) {
        return Some(EditorLauncher::MacApp(path));
    }

    #[cfg(windows)]
    if let Some(path) = installed_windows_app(editor) {
        return Some(EditorLauncher::Command(path));
    }

    editor
        .commands
        .iter()
        .find_map(|command| resolve_gui_binary(command))
        .or_else(|| toolbox_script(&toolbox_scripts_dir()?, editor))
        .map(EditorLauncher::Command)
}

/// Editors found on this machine. Blocking: the first call may read the
/// login shell's PATH, so callers run it on a background executor.
pub fn installed() -> Vec<ExternalEditor> {
    EDITORS
        .iter()
        .filter(|editor| resolve_editor(editor).is_some())
        .map(|editor| ExternalEditor {
            id: editor.id,
            name: editor.name,
        })
        .collect()
}

/// Opens the folder `cwd` in the editor `editor_id`. Blocking, like [`installed`].
pub fn open(editor_id: &str, cwd: &str) -> Result<(), String> {
    let editor = definition(editor_id).ok_or_else(|| "Unknown external editor.".to_string())?;
    let cwd = expand_home(Path::new(cwd));
    if !cwd.is_dir() {
        return Err(format!("{} is not a folder.", cwd.display()));
    }
    let launcher =
        resolve_editor(editor).ok_or_else(|| format!("{} is no longer installed.", editor.name))?;

    #[cfg(target_os = "macos")]
    let mut command = match launcher {
        EditorLauncher::MacApp(app) => {
            let mut command = Command::new("/usr/bin/open");
            command.arg("-a").arg(app).arg(&cwd);
            command
        }
        EditorLauncher::Command(program) => {
            let mut command = Command::new(program);
            command.arg(&cwd);
            command
        }
    };

    #[cfg(not(target_os = "macos"))]
    let mut command = match launcher {
        EditorLauncher::Command(program) => {
            let mut command = Command::new(program);
            command.arg(&cwd);
            command
        }
    };

    apply_gui_env(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open {}: {error}", editor.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_ids_and_names_are_unique() {
        let mut ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for editor in EDITORS {
            assert!(ids.insert(editor.id), "duplicate editor id: {}", editor.id);
            assert!(
                names.insert(editor.name),
                "duplicate editor name: {}",
                editor.name
            );
            assert!(!editor.commands.is_empty());
        }
    }

    #[test]
    fn unknown_editor_is_rejected_before_launch() {
        let error = open("not-an-editor", ".").unwrap_err();
        assert_eq!(error, "Unknown external editor.");
    }

    #[test]
    fn jetbrains_ides_are_supported() {
        for id in [
            "intellij-idea",
            "webstorm",
            "pycharm",
            "goland",
            "rustrover",
            "clion",
            "phpstorm",
            "rider",
            "rubymine",
            "android-studio",
        ] {
            assert!(definition(id).is_some(), "missing editor: {id}");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rebase-editor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn versioned_install_folder_resolves_to_the_newest_match() {
        let root = scratch("versioned");
        touch(&root.join("JetBrains/WebStorm 2024.3/bin/webstorm64.exe"));
        touch(&root.join("JetBrains/WebStorm 2025.2/bin/webstorm64.exe"));
        // A newer folder without the launcher is a leftover, not an install.
        std::fs::create_dir_all(root.join("JetBrains/WebStorm 2025.3/bin")).unwrap();
        touch(&root.join("JetBrains/GoLand 2026.1/bin/webstorm64.exe"));

        assert_eq!(
            installed_file(&root, "JetBrains/WebStorm*/bin/webstorm64.exe"),
            Some(root.join("JetBrains/WebStorm 2025.2/bin/webstorm64.exe"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_paths_without_a_wildcard_must_exist() {
        let root = scratch("fixed");
        touch(&root.join("Programs/Rider/bin/rider64.exe"));

        assert_eq!(
            installed_file(&root, "Programs/Rider/bin/rider64.exe"),
            Some(root.join("Programs/Rider/bin/rider64.exe"))
        );
        assert_eq!(
            installed_file(&root, "Programs/CLion/bin/clion64.exe"),
            None
        );
        assert_eq!(
            installed_file(&root, "Missing/CLion*/bin/clion64.exe"),
            None,
            "an absent parent folder is not an install"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn toolbox_scripts_are_found_by_command_name() {
        let dir = scratch("toolbox");
        let webstorm = definition("webstorm").unwrap();
        assert_eq!(toolbox_script(&dir, webstorm), None);

        let script = if cfg!(windows) {
            dir.join("webstorm.cmd")
        } else {
            dir.join("webstorm")
        };
        touch(&script);
        assert_eq!(toolbox_script(&dir, webstorm), Some(script));
        assert_eq!(toolbox_script(&dir, definition("goland").unwrap()), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
