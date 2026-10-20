use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaCategory {
    Video,
    Audio,
    Image,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConversionTier {
    Copy,
    EncodeHq,
}

pub const VIDEO_INPUTS: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "ts", "flv", "m4v", "wmv", "mpeg", "mpg", "3gp", "m3u8",
];
pub const AUDIO_INPUTS: &[&str] = &[
    "mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus", "aiff",
];
pub const IMAGE_INPUTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif", "ico", "avif", "jxl",
];

pub const VIDEO_OUTPUTS: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "ts", "gif"];
pub const AUDIO_OUTPUTS: &[&str] = &["mp3", "wav", "flac", "aac", "ogg", "m4a"];
pub const IMAGE_OUTPUTS: &[&str] = &["jpg", "png", "webp", "gif", "bmp", "tiff", "ico", "avif"];

pub fn normalize_ext(ext: &str) -> String {
    ext.trim_start_matches('.').to_lowercase()
}

pub fn extension_category(ext: &str) -> MediaCategory {
    let ext = normalize_ext(ext);
    if VIDEO_INPUTS.contains(&ext.as_str()) {
        MediaCategory::Video
    } else if AUDIO_INPUTS.contains(&ext.as_str()) {
        MediaCategory::Audio
    } else if IMAGE_INPUTS.contains(&ext.as_str()) {
        MediaCategory::Image
    } else {
        MediaCategory::Unknown
    }
}

pub fn is_supported_input(ext: &str) -> bool {
    extension_category(ext) != MediaCategory::Unknown
}

pub fn outputs_for_category(category: MediaCategory) -> Vec<String> {
    match category {
        MediaCategory::Video => VIDEO_OUTPUTS.iter().map(|s| s.to_string()).collect(),
        MediaCategory::Audio => AUDIO_OUTPUTS.iter().map(|s| s.to_string()).collect(),
        MediaCategory::Image => IMAGE_OUTPUTS.iter().map(|s| s.to_string()).collect(),
        MediaCategory::Unknown => vec![],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatGroup {
    pub label: String,
    pub formats: Vec<String>,
}

pub fn outputs_for_input(category: MediaCategory) -> Vec<FormatGroup> {
    match category {
        MediaCategory::Video => vec![
            FormatGroup {
                label: "Video".to_string(),
                formats: VIDEO_OUTPUTS.iter().map(|s| s.to_string()).collect(),
            },
            FormatGroup {
                label: "Audio (extract)".to_string(),
                formats: AUDIO_OUTPUTS.iter().map(|s| s.to_string()).collect(),
            },
        ],
        MediaCategory::Audio => vec![FormatGroup {
            label: "Audio".to_string(),
            formats: AUDIO_OUTPUTS.iter().map(|s| s.to_string()).collect(),
        }],
        MediaCategory::Image => vec![FormatGroup {
            label: "Image".to_string(),
            formats: IMAGE_OUTPUTS.iter().map(|s| s.to_string()).collect(),
        }],
        MediaCategory::Unknown => vec![],
    }
}

pub fn all_supported_extensions() -> Vec<String> {
    let mut exts: Vec<String> = VIDEO_INPUTS
        .iter()
        .chain(AUDIO_INPUTS.iter())
        .chain(IMAGE_INPUTS.iter())
        .map(|s| s.to_string())
        .collect();
    exts.sort();
    exts.dedup();
    exts
}

pub fn is_valid_output_for_input(input_category: MediaCategory, output_ext: &str) -> bool {
    let output_ext = normalize_ext(output_ext);
    match input_category {
        MediaCategory::Video => {
            VIDEO_OUTPUTS.contains(&output_ext.as_str())
                || AUDIO_OUTPUTS.contains(&output_ext.as_str())
        }
        MediaCategory::Audio => AUDIO_OUTPUTS.contains(&output_ext.as_str()),
        MediaCategory::Image => IMAGE_OUTPUTS.contains(&output_ext.as_str()),
        MediaCategory::Unknown => false,
    }
}

pub fn default_output_format(category: MediaCategory) -> String {
    match category {
        MediaCategory::Video => "mp4".to_string(),
        MediaCategory::Audio => "mp3".to_string(),
        MediaCategory::Image => "png".to_string(),
        MediaCategory::Unknown => "mp4".to_string(),
    }
}

pub fn is_lossless_extension(ext: &str) -> bool {
    let ext = normalize_ext(ext);
    matches!(
        ext.as_str(),
        "png" | "bmp" | "tiff" | "tif" | "flac" | "wav" | "aiff" | "alac"
    )
}

pub fn output_category(output_ext: &str) -> MediaCategory {
    let ext = normalize_ext(output_ext);
    if VIDEO_OUTPUTS.contains(&ext.as_str()) {
        MediaCategory::Video
    } else if AUDIO_OUTPUTS.contains(&ext.as_str()) {
        MediaCategory::Audio
    } else if IMAGE_OUTPUTS.contains(&ext.as_str()) {
        MediaCategory::Image
    } else {
        MediaCategory::Unknown
    }
}

pub fn video_codec_supported_in_container(codec: &str, container: &str) -> bool {
    let codec = codec.to_lowercase();
    let container = normalize_ext(container);
    match container.as_str() {
        "mp4" | "m4v" | "mov" => matches!(
            codec.as_str(),
            "h264" | "hevc" | "h265" | "mpeg4" | "av1"
        ),
        "webm" => matches!(codec.as_str(), "vp8" | "vp9" | "av1"),
        "mkv" => true,
        "avi" => matches!(
            codec.as_str(),
            "h264" | "hevc" | "h265" | "mpeg4" | "mpeg2video" | "mjpeg" | "vp9" | "vp8"
        ),
        "ts" => matches!(
            codec.as_str(),
            "h264" | "hevc" | "h265" | "mpeg2video" | "mpeg1video"
        ),
        "flv" => matches!(codec.as_str(), "h264" | "flv1"),
        "gif" => codec == "gif",
        _ => false,
    }
}

pub fn audio_codec_supported_in_container(codec: &str, container: &str) -> bool {
    let codec = codec.to_lowercase();
    let container = normalize_ext(container);
    match container.as_str() {
        "mp4" | "m4v" | "mov" | "m4a" => matches!(
            codec.as_str(),
            "aac" | "mp3" | "ac3" | "eac3" | "alac"
        ),
        "webm" => matches!(codec.as_str(), "opus" | "vorbis"),
        "mkv" => true,
        "avi" => matches!(codec.as_str(), "mp3" | "aac" | "ac3" | "pcm_s16le" | "pcm_s24le"),
        "ts" => matches!(
            codec.as_str(),
            "aac" | "ac3" | "mp3" | "mp2" | "pcm_s16le"
        ),
        "flv" => matches!(codec.as_str(), "aac" | "mp3"),
        "mp3" => codec == "mp3",
        "wav" => matches!(
            codec.as_str(),
            "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_s16be" | "flac"
        ),
        "flac" => codec == "flac",
        "aac" => codec == "aac",
        "ogg" => matches!(codec.as_str(), "vorbis" | "opus" | "flac"),
        "gif" => false,
        _ => false,
    }
}

pub fn audio_codec_matches_output(codec: &str, output_ext: &str) -> bool {
    let codec = codec.to_lowercase();
    let output = normalize_ext(output_ext);
    match output.as_str() {
        "mp3" => codec == "mp3",
        "flac" => codec == "flac",
        "aac" => codec == "aac",
        "ogg" => matches!(codec.as_str(), "vorbis" | "opus"),
        "m4a" => matches!(codec.as_str(), "aac" | "alac"),
        "wav" => matches!(
            codec.as_str(),
            "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le"
        ),
        _ => false,
    }
}
