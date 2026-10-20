use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

use crate::process_utils::hidden_command;

struct FfmpegTools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    bundled: bool,
}

static TOOLS: OnceLock<FfmpegTools> = OnceLock::new();

pub fn init(app: &AppHandle) {
    let resource = app.path().resolve("ffmpeg", BaseDirectory::Resource).ok();
    init_with_dirs(resource.into_iter().collect());
}

/// CLI entry: no Tauri runtime, so look next to the running executable.
pub fn init_standalone() {
    init_with_dirs(standalone_tool_dirs());
}

/// Folders checked for bundled tools when running without Tauri: `<exe>/ffmpeg`, the
/// executable's own folder, and `<exe>/../ffmpeg` (installed CLI lives in `<install>\cli`).
pub fn standalone_tool_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        dirs.push(exe_dir.join("ffmpeg"));
        if let Some(parent) = exe_dir.parent() {
            dirs.push(parent.join("ffmpeg"));
        }
        dirs.push(exe_dir);
    }
    dirs
}

pub fn dev_bin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/win64")
}

fn init_with_dirs(mut dirs: Vec<PathBuf>) {
    if TOOLS.get().is_some() {
        return;
    }
    dirs.push(dev_bin_dir());
    let tools = dirs
        .iter()
        .find_map(|d| resolve_bundled_dir(d))
        .unwrap_or_else(|| FfmpegTools {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
            bundled: false,
        });
    let _ = TOOLS.set(tools);
}

pub fn ffmpeg_path() -> PathBuf {
    TOOLS
        .get()
        .map(|t| t.ffmpeg.clone())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

pub fn ffprobe_path() -> PathBuf {
    TOOLS
        .get()
        .map(|t| t.ffprobe.clone())
        .unwrap_or_else(|| PathBuf::from("ffprobe"))
}

/// Fast path for bundled tools: verify files exist without spawning subprocesses.
pub fn check_available() -> Result<(), String> {
    let tools = TOOLS.get().ok_or_else(|| "FFmpeg is not initialized.".to_string())?;

    if tools.bundled {
        if tools.ffmpeg.is_file() && tools.ffprobe.is_file() {
            return Ok(());
        }
        return Err("Bundled FFmpeg binaries are missing.".to_string());
    }

    verify_on_path(&tools.ffmpeg, "FFmpeg")?;
    verify_on_path(&tools.ffprobe, "ffprobe")?;
    Ok(())
}

fn verify_on_path(program: &PathBuf, label: &str) -> Result<(), String> {
    let output = hidden_command(program)
        .arg("-version")
        .output()
        .map_err(|e| format!("{label} not found: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{label} is installed but returned an error."))
    }
}

fn resolve_bundled_dir(dir: &PathBuf) -> Option<FfmpegTools> {
    let ffmpeg = dir.join("ffmpeg.exe");
    let ffprobe = dir.join("ffprobe.exe");
    if ffmpeg.is_file() && ffprobe.is_file() {
        return Some(FfmpegTools {
            ffmpeg,
            ffprobe,
            bundled: true,
        });
    }
    None
}
