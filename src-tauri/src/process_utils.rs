use std::path::Path;
use std::process::{Command, Stdio};

/// Spawn a child process without flashing a console window on Windows.
pub fn hidden_command(program: impl AsRef<Path>) -> Command {
    let mut cmd = Command::new(program.as_ref());
    cmd.stdin(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}

/// Like `hidden_command`, at below-normal CPU priority so long encodes keep the desktop responsive.
pub fn hidden_background_command(program: impl AsRef<Path>) -> Command {
    let mut cmd = hidden_command(program);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }

    cmd
}
