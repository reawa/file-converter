use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

use crate::process_utils::hidden_command;

struct YtdlpTools {
    ytdlp: PathBuf,
    bundled: bool,
}

static TOOLS: OnceLock<YtdlpTools> = OnceLock::new();
static JS_RUNTIME: OnceLock<Option<(String, PathBuf)>> = OnceLock::new();

pub fn init(app: &AppHandle) {
    let resource = app.path().resolve("ffmpeg", BaseDirectory::Resource).ok();
    init_with_dirs(resource.into_iter().collect());
    spawn_self_update();
}

/// CLI entry: no Tauri runtime, so look next to the running executable.
pub fn init_standalone() {
    init_with_dirs(crate::ffmpeg_tools::standalone_tool_dirs());
}

/// Keep extractor up to date — stale yt-dlp is a common cause of YouTube 403s.
pub fn self_update_blocking() {
    let _ = hidden_command(ytdlp_path())
        .args(["-U", "--no-warnings"])
        .output();
}

fn spawn_self_update() {
    std::thread::spawn(|| {
        self_update_blocking();
        let _ = detect_js_runtime();
    });
}

pub fn ytdlp_path() -> PathBuf {
    TOOLS
        .get()
        .map(|t| t.ytdlp.clone())
        .unwrap_or_else(|| PathBuf::from("yt-dlp"))
}

pub fn ffmpeg_location() -> PathBuf {
    crate::ffmpeg_tools::ffmpeg_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Returns `(runtime_name, executable_path)` for yt-dlp `--js-runtimes`.
pub fn js_runtime() -> Option<(String, PathBuf)> {
    detect_js_runtime().clone()
}

fn detect_js_runtime() -> &'static Option<(String, PathBuf)> {
    JS_RUNTIME.get_or_init(|| {
        for (name, candidates) in [
            ("deno", &["deno.exe", "deno"][..]),
            ("node", &["node.exe", "node"][..]),
            ("bun", &["bun.exe", "bun"][..]),
        ] {
            if let Some(path) = find_on_path(candidates) {
                return Some((name.to_string(), path));
            }
        }
        None
    })
}

fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    for name in names {
        let output = hidden_command("where.exe").arg(name).output().ok()?;
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let first = text.lines().next()?.trim();
        if !first.is_empty() {
            let path = PathBuf::from(first);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Fast path for bundled tools: verify files exist without spawning subprocesses.
pub fn check_available() -> Result<(), String> {
    let tools = TOOLS
        .get()
        .ok_or_else(|| "yt-dlp is not initialized.".to_string())?;

    if tools.bundled {
        if tools.ytdlp.is_file() {
            return Ok(());
        }
        return Err("Bundled yt-dlp binary is missing.".to_string());
    }

    let output = hidden_command(&tools.ytdlp)
        .arg("--version")
        .output()
        .map_err(|e| format!("yt-dlp not found: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err("yt-dlp is installed but returned an error.".to_string())
    }
}

pub fn require_js_runtime_for_youtube() -> Result<(String, PathBuf), String> {
    js_runtime().ok_or_else(|| {
        "YouTube downloads need Node.js or Deno installed and on PATH (required by yt-dlp). Install Node from https://nodejs.org then restart the app.".to_string()
    })
}

fn init_with_dirs(mut dirs: Vec<PathBuf>) {
    if TOOLS.get().is_some() {
        return;
    }
    dirs.push(crate::ffmpeg_tools::dev_bin_dir());
    let tools = dirs
        .iter()
        .find_map(|d| resolve_bundled_dir(d))
        .unwrap_or_else(|| YtdlpTools {
            ytdlp: PathBuf::from("yt-dlp"),
            bundled: false,
        });
    let _ = TOOLS.set(tools);
}

fn resolve_bundled_dir(dir: &PathBuf) -> Option<YtdlpTools> {
    let ytdlp = dir.join("yt-dlp.exe");
    if ytdlp.is_file() {
        return Some(YtdlpTools {
            ytdlp,
            bundled: true,
        });
    }
    None
}
