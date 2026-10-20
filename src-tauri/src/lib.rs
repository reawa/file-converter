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

use std::sync::Arc;

use commands::{
    cancel_conversion, check_ffmpeg, check_output_conflicts, get_default_output_dir,
    get_output_format_groups, get_output_formats, get_supported_extensions, open_output_path,
    probe_files, reveal_output_path, start_conversion, update_job_format, update_job_hls_range,
    update_job_youtube_quality,
};
use queue::QueueState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let queue_state = Arc::new(QueueState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
