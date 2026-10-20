use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::format_registry::normalize_ext;
use crate::process_utils::hidden_command;
use crate::ytdlp_tools;

const MAX_URL_LEN: usize = 4096;

/// Append yt-dlp flags needed for modern YouTube (JS challenge + retries).
///
/// YouTube's JS challenge solver (`ejs`) is only enabled when
/// `FILE_CONVERTER_YTDLP_REMOTE_EJS` is unset or not `0`/`false`. Set it to `0`
/// to block yt-dlp from fetching remote challenge code (YouTube may then fail).
fn push_youtube_compat_args(args: &mut Vec<String>) -> Result<(), String> {
    let (runtime, path) = ytdlp_tools::require_js_runtime_for_youtube()?;
    args.push("--js-runtimes".into());
    // Explicit path so GUI-spawned processes still find Node/Deno.
    args.push(format!("{runtime}:{}", path.to_string_lossy().replace('\\', "/")));
    if remote_ejs_allowed() {
        args.push("--remote-components".into());
        args.push("ejs:github".into());
    }
    args.push("--retries".into());
    args.push("5".into());
    Ok(())
}

fn remote_ejs_allowed() -> bool {
    match std::env::var("FILE_CONVERTER_YTDLP_REMOTE_EJS") {
        Ok(v) => {
            let lower = v.trim().to_ascii_lowercase();
            !(lower == "0" || lower == "false" || lower == "off" || lower == "no")
        }
        Err(_) => true,
    }
}

static META_CACHE: LazyLock<Mutex<HashMap<String, YouTubeMeta>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeQuality {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeMeta {
    pub title: String,
    pub platform: String,
    pub qualities: Vec<YouTubeQuality>,
}

/// True for any http(s) URL that should be handled by yt-dlp (not raw HLS playlists).
pub fn is_ytdlp_url(path: &str) -> bool {
    let trimmed = path.trim().to_lowercase();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return false;
    }
    // Direct playlist files stay on the FFmpeg HLS path.
    if trimmed.contains(".m3u8") {
        return false;
    }
    true
}

/// True when the path is an http(s) YouTube watch/share URL (needs JS runtime).
pub fn is_youtube_url(path: &str) -> bool {
    let trimmed = path.trim().to_lowercase();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return false;
    }
    trimmed.contains("youtube.com/")
        || trimmed.contains("youtu.be/")
        || trimmed.contains("music.youtube.com/")
}

fn reject_embedded_credentials(url: &str) -> Result<(), String> {
    if let Some(rest) = url.split("://").nth(1) {
        if let Some(at) = rest.find('@') {
            let before = &rest[..at];
            if before.contains(':') && !before.contains('/') {
                return Err(
                    "Stream URLs with embedded credentials are not supported.".to_string(),
                );
            }
        }
    }
    Ok(())
}

/// Normalize known site quirks before handing the URL to yt-dlp.
pub fn normalize_remote_url(url: &str) -> String {
    let trimmed = url.trim().to_string();
    let lower = trimmed.to_lowercase();
    // Medal interstitial bypass (historical site quirk; harmless if already present).
    if lower.contains("medal.tv") && !lower.contains("mobilebypass") {
        if trimmed.contains('?') {
            format!("{trimmed}&mobilebypass=true")
        } else {
            format!("{trimmed}?mobilebypass=true")
        }
    } else {
        trimmed
    }
}

pub fn validate_ytdlp_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("Invalid URL.".to_string());
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err("URL is too long.".to_string());
    }
    if !is_ytdlp_url(trimmed) {
        return Err("Only http(s) media URLs are supported for remote downloads.".to_string());
    }
    reject_embedded_credentials(trimmed)?;
    Ok(())
}

pub fn platform_label_from_host(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    let host = lower
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .trim_start_matches("m.")
        .trim_start_matches("mobile.");
    match host {
        h if h.contains("youtube") || h == "youtu.be" => "YouTube".into(),
        h if h.contains("twitch") => "Twitch".into(),
        h if h.contains("instagram") => "Instagram".into(),
        h if h.contains("medal.tv") => "Medal".into(),
        h if h.contains("tiktok") => "TikTok".into(),
        h if h == "x.com" || h.contains("twitter") => "X".into(),
        h if h.contains("reddit") => "Reddit".into(),
        h if h.contains("facebook") || h.contains("fb.watch") => "Facebook".into(),
        h if h.contains("vimeo") => "Vimeo".into(),
        h if h.contains("soundcloud") => "SoundCloud".into(),
        h if h.contains("rumble") => "Rumble".into(),
        h if h.contains("bilibili") => "Bilibili".into(),
        h if h.contains("streamable") => "Streamable".into(),
        h if h.contains("kick.com") => "Kick".into(),
        h if h.contains("dailymotion") || h.contains("dai.ly") => "Dailymotion".into(),
        h if h.contains("pinterest") => "Pinterest".into(),
        h if h.contains("linkedin") => "LinkedIn".into(),
        h if h.contains("bandcamp") => "Bandcamp".into(),
        h if h.contains("nicovideo") || h.contains("nico.ms") => "Niconico".into(),
        h if h.contains("imgur") => "Imgur".into(),
        h if h.contains("clips.twitch") => "Twitch".into(),
        h if !h.is_empty() => {
            let base = h.split('.').next().unwrap_or(h);
            let mut chars = base.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => "Link".into(),
            }
        }
        _ => "Link".into(),
    }
}

fn friendly_extractor_name(extractor: &str) -> String {
    let e = extractor.trim();
    if e.is_empty() || e.eq_ignore_ascii_case("generic") {
        return "Link".into();
    }
    // TwitchClips → Twitch, InstagramIE-style keys already short
    let stripped = e
        .strip_suffix("IE")
        .unwrap_or(e)
        .strip_suffix("Clip")
        .or_else(|| e.strip_suffix("Clips"))
        .unwrap_or(e);
    let stripped = stripped.strip_suffix("TV").unwrap_or(stripped);
    match stripped.to_lowercase().as_str() {
        "youtube" | "youtube tab" => "YouTube".into(),
        "twitch" | "twitchclips" | "twitchvod" => "Twitch".into(),
        "instagram" => "Instagram".into(),
        "medal" | "medaltv" => "Medal".into(),
        "tiktok" => "TikTok".into(),
        "twitter" => "X".into(),
        "reddit" => "Reddit".into(),
        "facebook" => "Facebook".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => "Link".into(),
            }
        }
    }
}

pub fn sanitize_title_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0' => '_',
            '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}' => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

pub fn display_name_for_url(url: &str) -> String {
    if let Ok(cache) = META_CACHE.lock() {
        if let Some(meta) = cache.get(url.trim()) {
            return sanitize_title_stem(&meta.title);
        }
    }
    platform_label_from_host(url)
}

pub fn output_stem_for_url(url: &str) -> String {
    if let Ok(cache) = META_CACHE.lock() {
        if let Some(meta) = cache.get(url.trim()) {
            return sanitize_title_stem(&meta.title);
        }
    }
    "download".to_string()
}

pub fn cached_meta(url: &str) -> Option<YouTubeMeta> {
    META_CACHE
        .lock()
        .ok()
        .and_then(|c| c.get(url.trim()).cloned())
}

pub fn fetch_meta(url: &str) -> Result<YouTubeMeta, String> {
    validate_ytdlp_url(url)?;
    let key = normalize_remote_url(url);

    if let Some(cached) = cached_meta(&key) {
        return Ok(cached);
    }
    // Also check original URL cache key for UI that still holds the pasted form.
    if let Some(cached) = cached_meta(url.trim()) {
        return Ok(cached);
    }

    ytdlp_tools::check_available()?;

    let mut cmd_args = vec![
        "-J".to_string(),
        "--no-playlist".to_string(),
        "--no-warnings".to_string(),
        "--skip-download".to_string(),
        "--retries".to_string(),
        "5".to_string(),
    ];
    if is_youtube_url(&key) {
        push_youtube_compat_args(&mut cmd_args)?;
    }
    cmd_args.push(key.clone());

    let output = hidden_command(ytdlp_tools::ytdlp_path())
        .args(&cmd_args)
        .output()
        .map_err(|e| format!("Failed to run yt-dlp: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let msg = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("yt-dlp failed")
            .trim();
        return Err(format!("Could not read this link: {msg}"));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid yt-dlp JSON: {e}"))?;

    let meta = parse_meta_json(&json, &key)?;
    if let Ok(mut cache) = META_CACHE.lock() {
        cache.insert(key, meta.clone());
        cache.insert(url.trim().to_string(), meta.clone());
    }
    Ok(meta)
}

pub fn parse_meta_json(json: &serde_json::Value, url: &str) -> Result<YouTubeMeta, String> {
    let extractor = json
        .get("extractor_key")
        .or_else(|| json.get("extractor"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let platform = {
        let from_extractor = friendly_extractor_name(extractor);
        if from_extractor == "Link" {
            platform_label_from_host(url)
        } else {
            from_extractor
        }
    };

    let title = json
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Media download")
        .to_string();

    let mut heights = BTreeSet::new();
    if let Some(formats) = json.get("formats").and_then(|f| f.as_array()) {
        for fmt in formats {
            let has_video = fmt
                .get("vcodec")
                .and_then(|v| v.as_str())
                .is_some_and(|c| c != "none");
            if !has_video {
                continue;
            }
            if let Some(h) = fmt.get("height").and_then(|h| h.as_u64()) {
                if h > 0 {
                    heights.insert(h);
                }
            }
        }
    }

    let mut qualities = vec![YouTubeQuality {
        label: "Best".to_string(),
        value: "best".to_string(),
    }];
    for h in heights.iter().rev() {
        qualities.push(YouTubeQuality {
            label: format!("{h}p"),
            value: h.to_string(),
        });
    }
    qualities.push(YouTubeQuality {
        label: "Audio only".to_string(),
        value: "audio".to_string(),
    });

    Ok(YouTubeMeta {
        title,
        platform,
        qualities,
    })
}

/// Build yt-dlp `-f` format selector for height presets / audio.
pub fn format_selector(quality: &str) -> String {
    let q = quality.trim().to_lowercase();
    match q.as_str() {
        "audio" => "bestaudio/best".to_string(),
        "best" | "" => "bestvideo*+bestaudio/best".to_string(),
        height if height.chars().all(|c| c.is_ascii_digit()) => {
            format!("bestvideo[height<={height}]+bestaudio/best[height<={height}]/best")
        }
        _ => "bestvideo*+bestaudio/best".to_string(),
    }
}

pub fn normalize_quality(quality: &str, meta: &YouTubeMeta) -> String {
    let q = quality.trim().to_lowercase();
    if q.is_empty() {
        return "best".to_string();
    }
    if meta.qualities.iter().any(|item| item.value == q) {
        return q;
    }
    "best".to_string()
}

/// Build yt-dlp args for a download into `output_path`.
pub fn build_download_args(url: &str, quality: &str, output_path: &Path) -> Result<Vec<String>, String> {
    validate_ytdlp_url(url)?;
    let url = normalize_remote_url(url);
    let out_ext = normalize_ext(
        output_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4"),
    );
    let parent = output_path
        .parent()
        .ok_or_else(|| "Invalid output path.".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create output directory: {e}"))?;

    let stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let template = parent.join(format!("{stem}.%(ext)s"));
    let template_str = template.to_string_lossy().replace('\\', "/");
    let ffmpeg_dir_str = ytdlp_tools::ffmpeg_location()
        .to_string_lossy()
        .to_string();

    let is_audio_out = matches!(
        out_ext.as_str(),
        "mp3" | "m4a" | "aac" | "opus" | "ogg" | "flac" | "wav"
    ) || quality.eq_ignore_ascii_case("audio");

    let selector = if is_audio_out {
        "bestaudio/best".to_string()
    } else {
        format_selector(quality)
    };

    let mut args: Vec<String> = vec![
        "--no-playlist".into(),
        "--no-warnings".into(),
        "--newline".into(),
        "--progress".into(),
        "--retries".into(),
        "5".into(),
        "--ffmpeg-location".into(),
        ffmpeg_dir_str,
        "-o".into(),
        template_str,
        "-f".into(),
        selector,
    ];
    if is_youtube_url(&url) {
        push_youtube_compat_args(&mut args)?;
    }

    if is_audio_out {
        args.push("-x".into());
        args.push("--audio-format".into());
        let audio_fmt = if out_ext == "aac" {
            "m4a".to_string()
        } else if out_ext == "ogg" {
            "vorbis".to_string()
        } else if matches!(out_ext.as_str(), "mp4" | "mkv" | "webm" | "avi" | "mov" | "ts" | "gif")
        {
            // Audio-only quality with a video container → default to mp3 extract.
            "mp3".to_string()
        } else {
            out_ext.clone()
        };
        args.push(audio_fmt.clone());
        if audio_fmt == "mp3" || out_ext == "mp3" {
            args.push("--audio-quality".into());
            args.push("320K".into());
        } else {
            args.push("--audio-quality".into());
            args.push("0".into());
        }
    } else {
        args.push("--merge-output-format".into());
        args.push(out_ext);
    }

    args.push(url.trim().to_string());
    Ok(args)
}

/// After yt-dlp exits successfully, ensure `output_path` exists (rename if needed).
pub fn finalize_download_output(output_path: &Path) -> Result<(), String> {
    if output_path.is_file() {
        return Ok(());
    }
    let parent = output_path
        .parent()
        .ok_or_else(|| "Invalid output path.".to_string())?;
    let stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("youtube");

    let written = std::fs::read_dir(parent)
        .map_err(|e| format!("Failed to read output directory: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s == stem)
                && p.is_file()
        });

    if let Some(found) = written {
        if found != output_path {
            std::fs::rename(&found, output_path)
                .or_else(|_| {
                    std::fs::copy(&found, output_path).and_then(|_| std::fs::remove_file(&found))
                })
                .map_err(|e| format!("Failed to finalize output file: {e}"))?;
        }
        return Ok(());
    }

    Err("yt-dlp finished but the output file was not found.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_youtube_urls() {
        assert!(is_youtube_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://music.youtube.com/watch?v=abc"));
        assert!(!is_youtube_url("https://vimeo.com/123"));
        assert!(!is_youtube_url("https://cdn.example.com/a.m3u8"));
    }

    #[test]
    fn detects_generic_ytdlp_urls() {
        assert!(is_ytdlp_url(
            "https://www.twitch.tv/shinyatheninja/clip/AliveTransparentMochaGOWSkull-k8L157WZI-qdCMQy"
        ));
        assert!(is_ytdlp_url("https://www.instagram.com/reel/abc123/"));
        assert!(is_ytdlp_url("https://medal.tv/games/valorant/clips/jTBFnLKdLy15K"));
        assert!(is_ytdlp_url("https://www.tiktok.com/@user/video/123"));
        assert!(!is_ytdlp_url("https://cdn.example.com/vod/index.m3u8"));
        assert!(!is_ytdlp_url(r"C:\Videos\clip.mp4"));
    }

    #[test]
    fn medal_url_gets_mobilebypass() {
        let n = normalize_remote_url("https://medal.tv/games/valorant/clips/abc");
        assert!(n.contains("mobilebypass=true"));
    }

    #[test]
    fn parses_qualities_from_json() {
        let json = serde_json::json!({
            "title": "Test Video",
            "extractor_key": "TwitchClips",
            "formats": [
                {"height": 1080, "vcodec": "avc1"},
                {"height": 720, "vcodec": "avc1"},
                {"height": 1080, "vcodec": "vp9"},
                {"height": null, "vcodec": "none", "acodec": "mp4a"},
            ]
        });
        let meta = parse_meta_json(&json, "https://www.twitch.tv/x/clip/y").unwrap();
        assert_eq!(meta.title, "Test Video");
        assert_eq!(meta.platform, "Twitch");
        assert_eq!(meta.qualities[0].value, "best");
        assert!(meta.qualities.iter().any(|q| q.value == "1080"));
        assert!(meta.qualities.iter().any(|q| q.value == "720"));
        assert_eq!(meta.qualities.last().unwrap().value, "audio");
    }

    #[test]
    fn format_selector_height_and_audio() {
        assert!(format_selector("1080").contains("height<=1080"));
        assert_eq!(format_selector("audio"), "bestaudio/best");
        assert!(format_selector("best").contains("bestvideo"));
    }

    #[test]
    fn sanitize_title_stem_strips_invalid() {
        assert_eq!(sanitize_title_stem("a:b/c|d"), "a_b_c_d");
        assert_eq!(sanitize_title_stem("   "), "download");
    }
}
