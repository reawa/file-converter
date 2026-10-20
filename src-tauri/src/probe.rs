use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::ffmpeg_tools;
use crate::hls::{
    effective_input_ext, is_hls_source, is_m3u8_url, media_input_path, protocol_whitelist_for,
};
use crate::process_utils::hidden_command;

use crate::format_registry::{
    audio_codec_matches_output, audio_codec_supported_in_container, extension_category,
    is_lossless_extension, normalize_ext, output_category, video_codec_supported_in_container,
    ConversionTier, MediaCategory,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub category: MediaCategory,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub duration_secs: Option<f64>,
    pub is_lossless_source: bool,
    pub has_video: bool,
    pub has_audio: bool,
}

static PROBE_CACHE: LazyLock<Mutex<HashMap<String, ProbeResult>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn is_attached_picture(stream: &serde_json::Value) -> bool {
    stream
        .get("disposition")
        .and_then(|d| d.get("attached_pic"))
        .and_then(|v| v.as_i64())
        == Some(1)
}

pub fn probe_file(path: &str) -> Result<ProbeResult, String> {
    let is_remote = is_m3u8_url(path);
    let path_obj = Path::new(path.trim());

    if !is_remote && !path_obj.exists() {
        return Err(format!("File not found: {path}"));
    }

    let ext = effective_input_ext(path);
    let category = extension_category(&ext);
    if category == MediaCategory::Unknown {
        return Err(format!("Unsupported file type: .{ext}"));
    }

    let mut cmd = hidden_command(ffmpeg_tools::ffprobe_path());
    if is_hls_source(path, &ext) {
        cmd.args(["-protocol_whitelist", protocol_whitelist_for(path)]);
    }
    let input = media_input_path(path);
    cmd.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_streams",
        "-show_format",
        &input,
    ]);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run ffprobe: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe failed for {path}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("Invalid ffprobe JSON: {e}"))?;

    let mut video_codec = None;
    let mut audio_codec = None;
    let mut has_video = false;
    let mut has_audio = false;

    if let Some(streams) = json.get("streams").and_then(|s| s.as_array()) {
        for stream in streams {
            let codec_type = stream
                .get("codec_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let codec_name = stream
                .get("codec_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match codec_type {
                "video" => {
                    if is_attached_picture(stream) {
                        continue;
                    }
                    if codec_name != "gif" || !has_video {
                        has_video = true;
                        video_codec = Some(codec_name.to_string());
                    }
                }
                "audio" => {
                    has_audio = true;
                    audio_codec = Some(codec_name.to_string());
                }
                _ => {}
            }
        }
    }

    // Trust extension for audio/image inputs so embedded cover art does not classify mp3 as video.
    let inferred_category = if category == MediaCategory::Image {
        MediaCategory::Image
    } else if category == MediaCategory::Audio {
        MediaCategory::Audio
    } else if has_video {
        MediaCategory::Video
    } else if has_audio {
        MediaCategory::Audio
    } else {
        category
    };

    let duration_secs = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|s| s.parse::<f64>().ok());

    let is_lossless_source = is_lossless_extension(&ext)
        || audio_codec
            .as_ref()
            .is_some_and(|c| c == "flac" || c.starts_with("pcm"));

    Ok(ProbeResult {
        category: inferred_category,
        video_codec,
        audio_codec,
        duration_secs,
        is_lossless_source,
        has_video,
        has_audio,
    })
}

/// Returns cached probe metadata when available (avoids repeated ffprobe on format changes).
pub fn probe_file_cached(path: &str) -> Result<ProbeResult, String> {
    if let Ok(cache) = PROBE_CACHE.lock() {
        if let Some(cached) = cache.get(path) {
            return Ok(cached.clone());
        }
    }
    let result = probe_file(path)?;
    if let Ok(mut cache) = PROBE_CACHE.lock() {
        cache.insert(path.to_string(), result.clone());
    }
    Ok(result)
}

pub fn decide_tier(
    probe: &ProbeResult,
    input_ext: &str,
    output_ext: &str,
) -> ConversionTier {
    let input_ext = normalize_ext(input_ext);
    let output_ext = normalize_ext(output_ext);
    let out_cat = output_category(&output_ext);

    if probe.category == MediaCategory::Image && out_cat == MediaCategory::Image {
        if input_ext == output_ext {
            return ConversionTier::Copy;
        }
        return ConversionTier::EncodeHq;
    }

    if output_ext == "gif" {
        return ConversionTier::EncodeHq;
    }

    if probe.category == MediaCategory::Video && out_cat == MediaCategory::Audio {
        if let Some(ref ac) = probe.audio_codec {
            if audio_codec_matches_output(ac, &output_ext)
                && audio_codec_supported_in_container(ac, &output_ext)
            {
                return ConversionTier::Copy;
            }
        }
        return ConversionTier::EncodeHq;
    }

    if probe.category == MediaCategory::Audio && out_cat == MediaCategory::Audio {
        if let Some(ref ac) = probe.audio_codec {
            if audio_codec_matches_output(ac, &output_ext)
                && audio_codec_supported_in_container(ac, &output_ext)
            {
                return ConversionTier::Copy;
            }
        }
        return ConversionTier::EncodeHq;
    }

    if probe.category == MediaCategory::Video && out_cat == MediaCategory::Video {
        if input_ext == output_ext {
            let video_ok = probe
                .video_codec
                .as_ref()
                .is_none_or(|vc| video_codec_supported_in_container(vc, &output_ext));
            let audio_ok = probe.audio_codec.as_ref().is_none_or(|ac| {
                audio_codec_supported_in_container(ac, &output_ext)
            });
            if video_ok && audio_ok {
                return ConversionTier::Copy;
            }
        } else {
            let video_ok = probe
                .video_codec
                .as_ref()
                .is_none_or(|vc| video_codec_supported_in_container(vc, &output_ext));
            let audio_ok = probe.audio_codec.as_ref().is_none_or(|ac| {
                audio_codec_supported_in_container(ac, &output_ext)
            });
            if video_ok && audio_ok {
                return ConversionTier::Copy;
            }
        }
        return ConversionTier::EncodeHq;
    }

    ConversionTier::EncodeHq
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_registry::MediaCategory;

    fn video_probe(codecs: (&str, &str)) -> ProbeResult {
        ProbeResult {
            category: MediaCategory::Video,
            video_codec: Some(codecs.0.to_string()),
            audio_codec: Some(codecs.1.to_string()),
            duration_secs: Some(1.0),
            is_lossless_source: false,
            has_video: true,
            has_audio: true,
        }
    }

    #[test]
    fn ts_to_mp4_uses_copy_tier() {
        let probe = video_probe(("h264", "aac"));
        let tier = decide_tier(&probe, "ts", "mp4");
        assert_eq!(tier, ConversionTier::Copy);
    }

    #[test]
    fn mp4_to_mp3_uses_encode_tier() {
        let probe = video_probe(("h264", "aac"));
        let tier = decide_tier(&probe, "mp4", "mp3");
        assert_eq!(tier, ConversionTier::EncodeHq);
    }

    #[test]
    fn video_can_output_audio_formats() {
        use crate::format_registry::is_valid_output_for_input;
        assert!(is_valid_output_for_input(MediaCategory::Video, "mp3"));
        assert!(is_valid_output_for_input(MediaCategory::Video, "mp4"));
        assert!(!is_valid_output_for_input(MediaCategory::Audio, "mp4"));
    }

    #[test]
    fn png_to_webp_uses_encode_tier() {
        let probe = ProbeResult {
            category: MediaCategory::Image,
            video_codec: Some("png".to_string()),
            audio_codec: None,
            duration_secs: None,
            is_lossless_source: true,
            has_video: true,
            has_audio: false,
        };
        let tier = decide_tier(&probe, "png", "webp");
        assert_eq!(tier, ConversionTier::EncodeHq);
    }

    #[test]
    fn png_to_png_uses_copy_tier() {
        let probe = ProbeResult {
            category: MediaCategory::Image,
            video_codec: Some("png".to_string()),
            audio_codec: None,
            duration_secs: None,
            is_lossless_source: true,
            has_video: true,
            has_audio: false,
        };
        let tier = decide_tier(&probe, "png", "png");
        assert_eq!(tier, ConversionTier::Copy);
    }

    #[test]
    fn png_to_jpg_uses_encode_tier() {
        let probe = ProbeResult {
            category: MediaCategory::Image,
            video_codec: Some("png".to_string()),
            audio_codec: None,
            duration_secs: None,
            is_lossless_source: true,
            has_video: true,
            has_audio: false,
        };
        let tier = decide_tier(&probe, "png", "jpg");
        assert_eq!(tier, ConversionTier::EncodeHq);
    }

    #[test]
    fn png_to_ico_uses_encode_tier() {
        let probe = ProbeResult {
            category: MediaCategory::Image,
            video_codec: Some("png".to_string()),
            audio_codec: None,
            duration_secs: None,
            is_lossless_source: true,
            has_video: true,
            has_audio: false,
        };
        let tier = decide_tier(&probe, "png", "ico");
        assert_eq!(tier, ConversionTier::EncodeHq);
    }

    #[test]
    fn mp3_with_cover_art_stays_audio_category() {
        let probe = ProbeResult {
            category: MediaCategory::Audio,
            video_codec: Some("mjpeg".to_string()),
            audio_codec: Some("mp3".to_string()),
            duration_secs: Some(180.0),
            is_lossless_source: false,
            has_video: true,
            has_audio: true,
        };
        assert_eq!(probe.category, MediaCategory::Audio);
        let tier = decide_tier(&probe, "mp3", "ogg");
        assert_eq!(tier, ConversionTier::EncodeHq);
    }
}
