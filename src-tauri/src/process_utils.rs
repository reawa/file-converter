use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Open a local file with the OS default association (no shell plugin).
pub fn open_path_default(path: &Path) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|_| "File not found.".to_string())?;
    if !path.is_file() {
        return Err("Path is not a regular file.".to_string());
    }

    #[cfg(windows)]
    {
        let path_str = path.to_string_lossy().into_owned();
        hidden_command("rundll32")
            .args(["url.dll,FileProtocolHandler", &path_str])
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        hidden_command("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
        Ok(())
    }
}

/// Reveal a local file in the system file manager.
pub fn reveal_path_in_file_manager(path: &Path) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|_| "File not found.".to_string())?;
    if !path.is_file() {
        return Err("Path is not a regular file.".to_string());
    }

    #[cfg(windows)]
    {
        // explorer `/select,` expects a Windows path; commas in names are rare but break this.
        let path_str = path.to_string_lossy().replace('/', "\\");
        if path_str.contains(',') {
            let parent = path
                .parent()
                .map(PathBuf::from)
                .ok_or_else(|| "File not found.".to_string())?;
            hidden_command("explorer")
                .arg(parent)
                .spawn()
                .map_err(|e| format!("Failed to reveal file: {e}"))?;
        } else {
            hidden_command("explorer")
                .arg(format!("/select,{path_str}"))
                .spawn()
                .map_err(|e| format!("Failed to reveal file: {e}"))?;
        }
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let parent = path
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| "File not found.".to_string())?;
        hidden_command("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("Failed to reveal file: {e}"))?;
        Ok(())
    }
}

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
