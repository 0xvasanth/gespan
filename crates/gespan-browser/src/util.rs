//! Small platform helpers the module needs from its host app, kept here so
//! the module stands on its own.

/// A helper process (`reg query`) runs without flashing a console window.
pub fn hide_window_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command;
}
