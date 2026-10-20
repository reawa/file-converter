pub mod cli;
mod commands;
mod converter;
mod ffmpeg_tools;
mod format_registry;
mod hls;
mod hls_playlist;
mod path_validation;
mod probe;
mod process_utils;
mod queue;
mod youtube;
mod ytdlp_tools;

use std::fs;
use std::sync::Arc;

use commands::{
    cancel_conversion, check_ffmpeg, check_output_conflicts, get_default_output_dir,
    get_output_format_groups, get_output_formats, get_supported_extensions, open_output_path,
    probe_files, reveal_output_path, start_conversion, update_job_format, update_job_hls_range,
    update_job_youtube_quality,
};
use queue::QueueState;
use tauri::{AppHandle, Manager};

/// Optional GitHub PAT for private-release updates (`%APPDATA%\com.stardvn.file-converter\update-token`).
#[tauri::command]
fn update_token(app: AppHandle) -> Result<Option<String>, String> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("update-token");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let token = raw.trim().to_string();
    Ok(if token.is_empty() { None } else { Some(token) })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let queue_state = Arc::new(QueueState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(queue_state)
        .setup(|app| {
            ffmpeg_tools::init(app.handle());
            ytdlp_tools::init(app.handle());
            crate::converter::init_encoder_cache_background();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_ffmpeg,
            get_output_formats,
            get_output_format_groups,
            get_supported_extensions,
            probe_files,
            update_job_format,
            update_job_hls_range,
            update_job_youtube_quality,
            check_output_conflicts,
            start_conversion,
            cancel_conversion,
            get_default_output_dir,
            open_output_path,
            reveal_output_path,
            update_token,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
