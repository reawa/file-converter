use std::path::{Component, Path, PathBuf};

use crate::format_registry::{is_valid_output_for_input, normalize_ext, MediaCategory};
use crate::hls::{is_m3u8_url, validate_m3u8_url};
use crate::probe::probe_file_cached;
use crate::youtube::{is_ytdlp_url, validate_ytdlp_url};

pub const MAX_PROBE_PATHS: usize = 200;
pub const MAX_QUEUE_JOBS: usize = 200;

/// User Downloads folder (`%USERPROFILE%\Downloads`). Created on demand by callers.
pub fn default_downloads_dir() -> PathBuf {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let trimmed = profile.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("Downloads");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("Downloads");
        }
    }
    PathBuf::from("Downloads")
}

pub fn validate_local_input_path(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("Invalid file path.".to_string());
    }

    let trimmed = path.trim();
    let lower = trimmed.to_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("file://")
    {
        return Err("Remote URLs are not supported.".to_string());
    }

    if trimmed.starts_with("\\\\") {
        return Err("UNC paths are not supported.".to_string());
    }

    if trimmed.contains("\\\\.\\") || trimmed.contains("//./") {
        return Err("Invalid file path.".to_string());
    }

    let path_obj = Path::new(trimmed);
    if path_obj
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("Invalid file path.".to_string());
    }

    let meta = std::fs::metadata(path_obj).map_err(|_| "File not found.".to_string())?;
    if !meta.is_file() {
        return Err("Path is not a regular file.".to_string());
    }

    Ok(path_obj.to_path_buf())
}

/// Validate a conversion input: local media, HLS (.m3u8), or any yt-dlp http(s) link.
pub fn validate_input_path(path: &str) -> Result<(), String> {
    if is_m3u8_url(path) {
        return validate_m3u8_url(path);
    }
    if is_ytdlp_url(path) {
        return validate_ytdlp_url(path);
    }
    validate_local_input_path(path)?;
    Ok(())
}

pub fn validate_output_dir(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("Invalid output directory.".to_string());
    }

    let trimmed = path.trim();
    let lower = trimmed.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Err("Invalid output directory.".to_string());
    }

    if trimmed.starts_with("\\\\") {
        return Err("UNC paths are not supported.".to_string());
    }

    let path_obj = Path::new(trimmed);
    if path_obj
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("Invalid output directory.".to_string());
    }

    let meta = std::fs::metadata(path_obj).map_err(|_| "Output directory not found.".to_string())?;
    if !meta.is_dir() {
        return Err("Output path is not a directory.".to_string());
    }

    std::fs::canonicalize(path_obj)
        .map(strip_verbatim_prefix)
        .map_err(|_| "Invalid output directory.".to_string())
}

/// `canonicalize` on Windows returns `\\?\C:\...`; drop that prefix so saved paths read
/// normally. `\\?\UNC\` paths are left alone.
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

pub fn validate_output_format(ext: &str) -> Result<String, String> {
    let normalized = normalize_ext(ext);
    if normalized.is_empty()
        || normalized.len() > 10
        || normalized.contains('/')
        || normalized.contains('\\')
        || normalized.contains("..")
        || normalized.contains(':')
    {
        return Err("Invalid output format.".to_string());
    }
    Ok(normalized)
}

/// Canonicalize a path for containment checks. When `path` does not exist yet (typical
/// for output files), canonicalize its parent and re-attach the file name.
fn canonicalize_for_containment(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return std::fs::canonicalize(path).map_err(|_| "Invalid output path.".to_string());
    }

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "Invalid output path.".to_string())?;
    let file_name = path
        .file_name()
        .ok_or_else(|| "Invalid output path.".to_string())?;

    let parent_canon =
        std::fs::canonicalize(parent).map_err(|_| "Invalid output directory.".to_string())?;
    Ok(parent_canon.join(file_name))
}

/// Strip Windows extended-length prefix so canonical and non-canonical paths compare equal.
fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
        return PathBuf::from(stripped);
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(s.as_ref())
    }
}

#[cfg(windows)]
fn path_has_prefix(path: &Path, prefix: &Path) -> bool {
    use std::path::Component;

    fn component_eq(a: Component<'_>, b: Component<'_>) -> bool {
        match (a, b) {
            (Component::Prefix(a), Component::Prefix(b)) => {
                a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
            }
            (Component::RootDir, Component::RootDir) => true,
            (Component::Normal(a), Component::Normal(b)) => a.eq_ignore_ascii_case(b),
            (a, b) => a.as_os_str() == b.as_os_str(),
        }
    }

    let mut path_iter = path.components();
    let mut prefix_iter = prefix.components();
    loop {
        match (prefix_iter.next(), path_iter.next()) {
            (None, Some(_)) | (None, None) => return true,
            (Some(_), None) => return false,
            (Some(p), Some(c)) if component_eq(p, c) => {}
            (Some(_), Some(_)) => return false,
        }
    }
}

#[cfg(not(windows))]
fn path_has_prefix(path: &Path, prefix: &Path) -> bool {
    path.starts_with(prefix)
}

pub fn validate_output_path_contained(output: &Path, root: &Path) -> Result<(), String> {
    let root_canon = canonicalize_for_containment(root)?;
    let output_canon = canonicalize_for_containment(output)?;

    let root_norm = normalize_path_for_comparison(&root_canon);
    let output_norm = normalize_path_for_comparison(&output_canon);

    if !path_has_prefix(&output_norm, &root_norm) {
        return Err("Output path escapes the selected output directory.".to_string());
    }

    Ok(())
}

pub fn validate_job_for_conversion(
    path: &str,
    output_format: &str,
) -> Result<(String, String), String> {
    validate_input_path(path)?;
    let format = validate_output_format(output_format)?;

    if is_ytdlp_url(path) {
        if !is_valid_output_for_input(MediaCategory::Video, &format)
            && !is_valid_output_for_input(MediaCategory::Audio, &format)
        {
            return Err(format!("Invalid output format .{format} for this link."));
        }
        return Ok((path.trim().to_string(), format));
    }

    let probe = probe_file_cached(path)?;
    if !is_valid_output_for_input(probe.category, &format) {
        return Err(format!("Invalid output format .{format} for this file."));
    }

    if probe.category == MediaCategory::Image
        && format == "avif"
        && !crate::converter::avif_output_allowed()
    {
        return Err(
            "AVIF output requires an AV1 encoder (libsvtav1 or libaom-av1) in your FFmpeg build."
                .to_string(),
        );
    }

    Ok((path.to_string(), format))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn output_dir_has_no_verbatim_prefix() {
        let dir = validate_output_dir(&std::env::temp_dir().to_string_lossy()).unwrap();
        assert!(!dir.to_string_lossy().starts_with(r"\\?\"), "{}", dir.display());
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\?\UNC\server\share")
        );
    }

    #[test]
    fn rejects_remote_in_local_validator_but_accepts_ytdlp_urls() {
        assert!(validate_local_input_path("https://evil.com/video.mp4").is_err());
        // Generic http(s) links are accepted for yt-dlp; unsupported sites fail at probe/download.
        assert!(validate_input_path("https://evil.com/video.mp4").is_ok());
        assert!(validate_input_path(
            "https://www.twitch.tv/x/clip/AliveTransparentMochaGOWSkull-k8L157WZI-qdCMQy"
        )
        .is_ok());
        assert!(validate_input_path("https://medal.tv/games/valorant/clips/abc").is_ok());
        assert!(validate_input_path("https://www.instagram.com/reel/abc/").is_ok());
    }

    #[test]
    fn accepts_m3u8_stream_urls() {
        assert!(validate_input_path("https://cdn.example.com/vod/index.m3u8").is_ok());
    }

    #[test]
    fn accepts_youtube_urls() {
        assert!(validate_input_path("https://www.youtube.com/watch?v=dQw4w9WgXcQ").is_ok());
        assert!(validate_input_path("https://youtu.be/dQw4w9WgXcQ").is_ok());
    }

    #[test]
    fn rejects_url_paths() {
        assert!(validate_local_input_path("https://evil.com/video.mp4").is_err());
    }

    #[test]
    fn rejects_traversal_in_output_format() {
        assert!(validate_output_format("..\\win.ini").is_err());
        assert!(validate_output_format("mp4").is_ok());
    }

    #[test]
    fn validates_existing_file() {
        let dir = std::env::temp_dir().join(format!("fc-val-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.mp4");
        fs::File::create(&file).unwrap().write_all(b"x").unwrap();
        assert!(validate_local_input_path(file.to_str().unwrap()).is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn output_in_same_dir_passes_containment() {
        let dir = std::env::temp_dir().join(format!("fc-contain-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        let input_name = "[FREE] Juice WRLD Type Beat - _We Dont Get Along_.mp3";
        let input = dir.join(input_name);
        fs::File::create(&input).unwrap().write_all(b"x").unwrap();

        let output = dir.join("[FREE] Juice WRLD Type Beat - _We Dont Get Along_.ogg");
        let root = dir.as_path();

        validate_output_path_contained(&output, root).expect("same-directory output should pass");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn output_with_quotes_and_brackets_passes_containment() {
        let dir = std::env::temp_dir().join(format!("fc-contain-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        // Curly quotes are valid on Windows; straight " would fail file creation.
        let output = dir.join("[free] lil peep type beat \u{201C}bridge\u{201D}.ogg");
        validate_output_path_contained(&output, dir.as_path())
            .expect("special characters in filename should not fail containment");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn output_outside_root_fails_containment() {
        let base = std::env::temp_dir().join(format!("fc-contain-test-{}", uuid::Uuid::new_v4()));
        let inside = base.join("inside");
        let outside = base.join("outside");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let output = outside.join("escaped.ogg");
        assert!(validate_output_path_contained(&output, inside.as_path()).is_err());

        let _ = fs::remove_dir_all(base);
    }
}
