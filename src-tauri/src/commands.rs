use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::converter::avif_output_allowed;
use crate::ffmpeg_tools;
use crate::format_registry::{
    all_supported_extensions, default_output_format, extension_category, is_supported_input,
    is_valid_output_for_input, normalize_ext, outputs_for_category, outputs_for_input,
    ConversionTier, FormatGroup, MediaCategory,
};
use crate::hls::{display_name_from_input, effective_input_ext, is_m3u8_url};
use crate::hls_playlist::{
    parse_range_from_url, parse_time_value, prepare_ranged_playlist, strip_url_time_params, HlsRange,
};
use crate::path_validation::{
    default_downloads_dir, validate_input_path, validate_job_for_conversion,
    validate_local_input_path, validate_output_dir, validate_output_format, MAX_PROBE_PATHS,
    MAX_QUEUE_JOBS,
};
use crate::probe::{decide_tier, probe_file_cached};
use crate::queue::{cancel, new_job_id, resolve_output_paths, run_queue, ConversionJob, QueueState};
use crate::youtube::{
    display_name_for_url, fetch_meta, is_ytdlp_url, normalize_quality, platform_label_from_host,
    YouTubeQuality,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileJobInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub category: MediaCategory,
    pub tier: ConversionTier,
    pub output_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hls_start_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hls_end_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube_quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube_qualities: Option<Vec<YouTubeQuality>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputConflict {
    pub id: String,
    pub output_path: String,
}

#[tauri::command]
pub fn check_ffmpeg() -> Result<(), String> {
    ffmpeg_tools::check_available()
}

#[tauri::command]
pub fn get_output_formats(category: MediaCategory) -> Vec<String> {
    outputs_for_category(category)
}

#[tauri::command]
pub fn get_output_format_groups(category: MediaCategory) -> Vec<FormatGroup> {
    outputs_for_input(category)
}

#[tauri::command]
pub fn get_supported_extensions() -> Vec<String> {
    all_supported_extensions()
}

#[tauri::command]
pub fn probe_files(
    paths: Vec<String>,
    output_format: Option<String>,
) -> Result<Vec<FileJobInfo>, String> {
    if paths.len() > MAX_PROBE_PATHS {
        return Err(format!("Too many files at once (max {MAX_PROBE_PATHS})."));
    }

    Ok(paths
        .par_iter()
        .map(|path| build_job_info(path.clone(), output_format.clone()))
        .collect())
}

fn empty_job_fields(
    path: String,
    name: String,
    category: MediaCategory,
    output_format: String,
    error: Option<String>,
) -> FileJobInfo {
    FileJobInfo {
        id: new_job_id(),
        path: path.clone(),
        name,
        category,
        tier: ConversionTier::EncodeHq,
        output_format,
        hls_start_secs: None,
        hls_end_secs: None,
        youtube_quality: None,
        youtube_qualities: None,
        platform: if is_ytdlp_url(&path) {
            Some(platform_label_from_host(&path))
        } else {
            None
        },
        error,
    }
}

fn build_remote_job_info(path: String, output_format: Option<String>) -> FileJobInfo {
    let clean = path.trim().to_string();
    match validate_input_path(&clean).and_then(|_| fetch_meta(&clean)) {
        Ok(meta) => {
            let name = meta.title.clone();
            let default_fmt = output_format.unwrap_or_else(|| "mp4".to_string());
            let fmt = if is_valid_output_for_input(MediaCategory::Video, &default_fmt) {
                validate_output_format(&default_fmt).unwrap_or_else(|_| "mp4".to_string())
            } else {
                "mp4".to_string()
            };
            FileJobInfo {
                id: new_job_id(),
                path: clean,
                name,
                category: MediaCategory::Video,
                tier: ConversionTier::EncodeHq,
                output_format: fmt,
                hls_start_secs: None,
                hls_end_secs: None,
                youtube_quality: Some("best".to_string()),
                youtube_qualities: Some(meta.qualities),
                platform: Some(meta.platform),
                error: None,
            }
        }
        Err(e) => empty_job_fields(
            clean.clone(),
            display_name_for_url(&clean),
            MediaCategory::Video,
            output_format.unwrap_or_else(|| "mp4".to_string()),
            Some(e),
        ),
    }
}

pub(crate) fn build_job_info(path: String, output_format: Option<String>) -> FileJobInfo {
    if is_ytdlp_url(&path) {
        return build_remote_job_info(path, output_format);
    }

    let url_range = parse_range_from_url(&path);
    let clean_path = if is_m3u8_url(&path) {
        strip_url_time_params(&path)
    } else {
        path.clone()
    };

    let name = display_name_from_input(&clean_path);
    let ext = effective_input_ext(&clean_path);

    if !is_supported_input(&ext) {
        let ext_display = if ext.is_empty() {
            "unknown".to_string()
        } else {
            ext.clone()
        };
        return FileJobInfo {
            id: new_job_id(),
            path: clean_path,
            name,
            category: MediaCategory::Unknown,
            tier: ConversionTier::EncodeHq,
            output_format: output_format.unwrap_or_else(|| "mp4".to_string()),
            hls_start_secs: url_range.start_secs,
            hls_end_secs: url_range.end_secs,
            youtube_quality: None,
            youtube_qualities: None,
            platform: None,
            error: Some(format!("Unsupported file type: .{ext_display}")),
        };
    }

    let category = extension_category(&ext);
    let default_fmt = output_format.unwrap_or_else(|| default_output_format(category));

    match validate_input_path(&clean_path).and_then(|_| probe_file_cached(&clean_path)) {
        Ok(probe) => {
            let fmt = if is_valid_output_for_input(probe.category, &default_fmt) {
                validate_output_format(&default_fmt)
                    .unwrap_or_else(|_| default_output_format(probe.category))
            } else {
                default_output_format(probe.category)
            };
            let tier = decide_tier(&probe, &ext, &fmt);
            FileJobInfo {
                id: new_job_id(),
                path: clean_path,
                name,
                category: probe.category,
                tier,
                output_format: fmt,
                hls_start_secs: url_range.start_secs,
                hls_end_secs: url_range.end_secs,
                youtube_quality: None,
                youtube_qualities: None,
                platform: None,
                error: None,
            }
        }
        Err(e) => FileJobInfo {
            id: new_job_id(),
            path: clean_path,
            name,
            category,
            tier: ConversionTier::EncodeHq,
            output_format: validate_output_format(&default_fmt).unwrap_or(default_fmt),
            hls_start_secs: url_range.start_secs,
            hls_end_secs: url_range.end_secs,
            youtube_quality: None,
            youtube_qualities: None,
            platform: None,
            error: Some(e),
        },
    }
}

#[tauri::command]
pub fn update_job_format(
    path: String,
    output_format: String,
    youtube_quality: Option<String>,
) -> Result<FileJobInfo, String> {
    validate_input_path(&path)?;
    let output_format = validate_output_format(&output_format)?;

    if is_ytdlp_url(&path) {
        let meta = fetch_meta(&path)?;
        let quality = normalize_quality(youtube_quality.as_deref().unwrap_or("best"), &meta);
        let category = if matches!(
            output_format.as_str(),
            "mp3" | "m4a" | "aac" | "opus" | "ogg" | "flac" | "wav"
        ) || quality == "audio"
        {
            MediaCategory::Audio
        } else {
            MediaCategory::Video
        };
        if !is_valid_output_for_input(MediaCategory::Video, &output_format)
            && !is_valid_output_for_input(MediaCategory::Audio, &output_format)
        {
            return Err(format!(
                "Invalid output format .{output_format} for this link"
            ));
        }
        return Ok(FileJobInfo {
            id: new_job_id(),
            path: path.clone(),
            name: meta.title,
            category,
            tier: ConversionTier::EncodeHq,
            output_format,
            hls_start_secs: None,
            hls_end_secs: None,
            youtube_quality: Some(quality),
            youtube_qualities: Some(meta.qualities),
            platform: Some(meta.platform),
            error: None,
        });
    }

    let name = display_name_from_input(&path);
    let ext = effective_input_ext(&path);
    let probe = probe_file_cached(&path)?;

    if !is_valid_output_for_input(probe.category, &output_format) {
        return Err(format!(
            "Invalid output format .{output_format} for this file"
        ));
    }

    if normalize_ext(&output_format) == "avif" && !avif_output_allowed() {
        return Err(
            "AVIF output requires an AV1 encoder (libsvtav1 or libaom-av1) in your FFmpeg build."
                .to_string(),
        );
    }

    let tier = decide_tier(&probe, &ext, &output_format);

    Ok(FileJobInfo {
        id: new_job_id(),
        path,
        name,
        category: probe.category,
        tier,
        output_format,
        hls_start_secs: None,
        hls_end_secs: None,
        youtube_quality: None,
        youtube_qualities: None,
        platform: None,
        error: None,
    })
}

#[tauri::command]
pub fn update_job_hls_range(
    path: String,
    output_format: String,
    hls_start: Option<String>,
    hls_end: Option<String>,
) -> Result<FileJobInfo, String> {
    validate_input_path(&path)?;
    let output_format = validate_output_format(&output_format)?;
    let start_secs = match hls_start {
        Some(s) => parse_time_value(&s)?,
        None => None,
    };
    let end_secs = match hls_end {
        Some(s) => parse_time_value(&s)?,
        None => None,
    };
    let range = HlsRange {
        start_secs,
        end_secs,
    };
    let ext = effective_input_ext(&path);
    if !is_m3u8_url(&path) && ext != "m3u8" {
        return Err("Start/end times apply to HLS (.m3u8) sources only.".to_string());
    }
    if range.is_active() {
        range.validate()?;
        let temp = prepare_ranged_playlist(&path, &range)?;
        let _ = std::fs::remove_file(temp);
    }
    let probe = probe_file_cached(&path)?;
    if !is_valid_output_for_input(probe.category, &output_format) {
        return Err(format!(
            "Invalid output format .{output_format} for this file"
        ));
    }
    let tier = decide_tier(&probe, &ext, &output_format);
    Ok(FileJobInfo {
        id: new_job_id(),
        path: path.clone(),
        name: display_name_from_input(&path),
        category: probe.category,
        tier,
        output_format,
        hls_start_secs: start_secs,
        hls_end_secs: end_secs,
        youtube_quality: None,
        youtube_qualities: None,
        platform: None,
        error: None,
    })
}

#[tauri::command]
pub fn update_job_youtube_quality(
    path: String,
    output_format: String,
    youtube_quality: String,
) -> Result<FileJobInfo, String> {
    validate_input_path(&path)?;
    if !is_ytdlp_url(&path) {
        return Err("Quality options apply to remote media links only.".to_string());
    }
    let mut output_format = validate_output_format(&output_format)?;
    let meta = fetch_meta(&path)?;
    let quality = normalize_quality(&youtube_quality, &meta);

    let category = if quality == "audio" {
        if !is_valid_output_for_input(MediaCategory::Audio, &output_format) {
            output_format = "mp3".to_string();
        }
        MediaCategory::Audio
    } else if is_valid_output_for_input(MediaCategory::Video, &output_format) {
        MediaCategory::Video
    } else if is_valid_output_for_input(MediaCategory::Audio, &output_format) {
        MediaCategory::Audio
    } else {
        output_format = "mp4".to_string();
        MediaCategory::Video
    };

    Ok(FileJobInfo {
        id: new_job_id(),
        path: path.clone(),
        name: meta.title,
        category,
        tier: ConversionTier::EncodeHq,
        output_format,
        hls_start_secs: None,
        hls_end_secs: None,
        youtube_quality: Some(quality),
        youtube_qualities: Some(meta.qualities),
        platform: Some(meta.platform),
        error: None,
    })
}

#[tauri::command]
pub fn check_output_conflicts(
    jobs: Vec<ConversionJob>,
    output_dir: Option<String>,
) -> Result<Vec<OutputConflict>, String> {
    let output_dir_path = match output_dir.as_deref() {
        Some(dir) if !dir.is_empty() => Some(validate_output_dir(dir)?),
        _ => None,
    };

    Ok(resolve_output_paths(
        &jobs,
        output_dir_path.as_ref().map(|p| p.as_path()),
    )
    .into_iter()
    .filter(|(_, path)| path.exists())
    .map(|(id, path)| OutputConflict {
        id,
        output_path: path.to_string_lossy().to_string(),
    })
    .collect())
}

/// Validate and normalize queued jobs. Shared by the window and the CLI.
pub(crate) fn validate_jobs(
    jobs: Vec<ConversionJob>,
    output_dir: Option<&str>,
    allow_overwrite: bool,
) -> Result<(Vec<ConversionJob>, Option<PathBuf>), String> {
    if jobs.is_empty() {
        return Err("No files to convert.".to_string());
    }
    if jobs.len() > MAX_QUEUE_JOBS {
        return Err(format!("Too many files in queue (max {MAX_QUEUE_JOBS})."));
    }

    let output_dir_path = match output_dir {
        Some(dir) if !dir.is_empty() => Some(validate_output_dir(dir)?),
        _ => None,
    };

    let mut validated_jobs = Vec::with_capacity(jobs.len());
    for job in jobs {
        let (path, output_format) =
            validate_job_for_conversion(&job.path, &job.output_format)?;
        let range = HlsRange {
            start_secs: job.hls_start_secs,
            end_secs: job.hls_end_secs,
        };
        if range.is_active() {
            range.validate()?;
        }
        let youtube_quality = if is_ytdlp_url(&path) {
            Some(
                job.youtube_quality
                    .as_deref()
                    .unwrap_or("best")
                    .trim()
                    .to_lowercase(),
            )
        } else {
            None
        };
        let output_name = job
            .output_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        validated_jobs.push(ConversionJob {
            id: job.id,
            path,
            output_format,
            hls_start_secs: job.hls_start_secs,
            hls_end_secs: job.hls_end_secs,
            youtube_quality,
            output_name,
        });
    }

    if !allow_overwrite {
        let conflicts: Vec<_> = resolve_output_paths(&validated_jobs, output_dir_path.as_deref())
            .into_iter()
            .filter(|(_, path)| path.exists())
            .collect();
        if !conflicts.is_empty() {
            return Err(format!(
                "{} output file(s) already exist. Confirm overwrite to continue.",
                conflicts.len()
            ));
        }
    }

    Ok((validated_jobs, output_dir_path))
}

#[tauri::command]
pub fn start_conversion(
    app: tauri::AppHandle,
    jobs: Vec<ConversionJob>,
    output_dir: Option<String>,
    allow_overwrite: bool,
    state: tauri::State<'_, Arc<QueueState>>,
) -> Result<(), String> {
    if state.running.load(Ordering::SeqCst) {
        return Err("A conversion is already running.".to_string());
    }
    let (validated_jobs, output_dir_path) =
        validate_jobs(jobs, output_dir.as_deref(), allow_overwrite)?;

    run_queue(
        Arc::new(app),
        validated_jobs,
        output_dir_path.map(|p| p.to_string_lossy().to_string()),
        state.inner().clone(),
    );
    Ok(())
}

#[tauri::command]
pub fn cancel_conversion(state: tauri::State<'_, Arc<QueueState>>) -> Result<(), String> {
    cancel(state.inner());
    Ok(())
}

/// Default save folder for remote/HLS jobs when no output folder is selected.
#[tauri::command]
pub fn get_default_output_dir() -> String {
    default_downloads_dir().to_string_lossy().to_string()
}

#[tauri::command]
pub fn open_output_path(path: String) -> Result<(), String> {
    validate_local_input_path(&path)?;
    Ok(())
}

#[tauri::command]
pub fn reveal_output_path(path: String) -> Result<(), String> {
    validate_local_input_path(&path)?;
    Ok(())
}
