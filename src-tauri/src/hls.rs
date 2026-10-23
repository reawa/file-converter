use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use crate::format_registry::{is_supported_input, normalize_ext};

/// Local `.m3u8` files (and temp ranged playlists) may reference `file:` + http(s).
pub const PROTOCOL_WHITELIST_LOCAL: &str = "file,http,https,tcp,tls,crypto";
/// Remote playlists must not allow `file:` (hostile `#EXTINF` / map URIs).
pub const PROTOCOL_WHITELIST_REMOTE: &str = "http,https,tcp,tls,crypto";

/// Protocol whitelist for ffmpeg/ffprobe based on whether the primary input is remote.
pub fn protocol_whitelist_for(input: &str) -> &'static str {
    if is_m3u8_url(input) {
        PROTOCOL_WHITELIST_REMOTE
    } else {
        PROTOCOL_WHITELIST_LOCAL
    }
}

const MAX_URL_LEN: usize = 4096;
const SNIFF_BYTES: usize = 4096;

/// True when the path is an http(s) URL pointing at an M3U8 playlist.
pub fn is_m3u8_url(path: &str) -> bool {
    let trimmed = path.trim();
    let lower = trimmed.to_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    lower.contains(".m3u8")
}

pub fn is_m3u8_extension(ext: &str) -> bool {
    normalize_ext(ext) == "m3u8"
}

/// True for local `.m3u8` files, playlist files sniffed as M3U8, or remote M3U8 URLs.
pub fn is_hls_source(path: &str, ext: &str) -> bool {
    is_m3u8_url(path) || is_m3u8_extension(ext) || sniff_m3u8_file(Path::new(path))
}

/// Normalize a local playlist path for ffmpeg/ffprobe (native path; bundled Windows ffmpeg rejects `file:///`).
pub fn media_input_path(path: &str) -> String {
    let trimmed = path.trim();
    if is_m3u8_url(trimmed) {
        return trimmed.to_string();
    }
    let path_obj = Path::new(trimmed);
    if !path_obj.exists() {
        return trimmed.to_string();
    }
    if !is_hls_source(trimmed, &effective_input_ext(trimmed)) {
        return trimmed.to_string();
    }
    let abs = fs::canonicalize(path_obj).unwrap_or_else(|_| path_obj.to_path_buf());
    let mut s = abs.to_string_lossy().to_string();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        s = stripped.to_string();
    }
    s.replace('\\', "/")
}

/// Read the start of a local file and check for an HLS playlist header.
pub fn sniff_m3u8_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; SNIFF_BYTES];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if n == 0 {
        return false;
    }
    let text = String::from_utf8_lossy(&buf[..n]);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    trimmed.starts_with("#EXTM3U")
}

pub fn validate_m3u8_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("Invalid stream URL.".to_string());
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err("Stream URL is too long.".to_string());
    }
    if !is_m3u8_url(trimmed) {
        return Err("Only http(s) .m3u8 playlist URLs are supported.".to_string());
    }
    let lower = trimmed.to_lowercase();
    if lower.starts_with("ftp://") || lower.starts_with("file://") {
        return Err("Only http(s) .m3u8 playlist URLs are supported.".to_string());
    }
    if trimmed.contains('@') {
        return Err("Stream URLs with embedded credentials are not supported.".to_string());
    }
    Ok(())
}

/// Resolve the input extension, including content sniffing for extensionless playlists.
pub fn effective_input_ext(path: &str) -> String {
    if is_m3u8_url(path) {
        return "m3u8".to_string();
    }
    let path_obj = Path::new(path);
    if let Some(ext) = path_obj.extension().and_then(|e| e.to_str()) {
        let normalized = normalize_ext(ext);
        if is_supported_input(&normalized) {
            return normalized;
        }
    }
    if sniff_m3u8_file(path_obj) {
        return "m3u8".to_string();
    }
    path_obj
        .extension()
        .and_then(|e| e.to_str())
        .map(normalize_ext)
        .unwrap_or_default()
}

pub fn display_name_from_input(path: &str) -> String {
    if is_m3u8_url(path) {
        let trimmed = path.trim();
        let segment = trimmed
            .split('/')
            .filter(|s| !s.is_empty())
            .last()
            .unwrap_or("stream.m3u8");
        let name = segment.split('?').next().unwrap_or(segment);
        return if name.is_empty() {
            "stream.m3u8".to_string()
        } else {
            name.to_string()
        };
    }
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

pub fn output_stem_from_input(input: &str) -> String {
    if is_m3u8_url(input) {
        let name = display_name_from_input(input);
        let stem = name
            .strip_suffix(".m3u8")
            .or_else(|| name.strip_suffix(".M3U8"))
            .unwrap_or(name.as_str());
        return if stem.is_empty() {
            "stream".to_string()
        } else {
            stem.to_string()
        };
    }
    Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("output")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_m3u8_urls() {
        assert!(is_m3u8_url(
            "https://cdn.example.com/vod/index-dvr.m3u8"
        ));
        assert!(!is_m3u8_url("https://cdn.example.com/video.mp4"));
        assert!(!is_m3u8_url("ftp://example.com/index.m3u8"));
    }

    #[test]
    fn rejects_credentials_in_url() {
        assert!(validate_m3u8_url("https://x.com/a.m3u8").is_ok());
        assert!(validate_m3u8_url("https://user:pass@x.com/a.m3u8").is_err());
    }

    #[test]
    fn sniffs_playlist_without_extension() {
        let dir = std::env::temp_dir().join(format!("fc-hls-sniff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("playlist");
        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"#EXTM3U\n#EXT-X-VERSION:3\n")
            .unwrap();
        assert!(sniff_m3u8_file(&file));
        assert_eq!(effective_input_ext(file.to_str().unwrap()), "m3u8");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn output_stem_from_url() {
        assert_eq!(
            output_stem_from_input("https://cdn.example.com/path/index-dvr.m3u8?token=1"),
            "index-dvr"
        );
    }

    #[test]
    fn remote_playlist_whitelist_excludes_file() {
        let remote = protocol_whitelist_for(
            "https://cdn.example.com/vod/index-dvr.m3u8",
        );
        assert!(!remote.split(',').any(|p| p == "file"));
        assert!(remote.contains("https"));
    }

    #[test]
    fn local_playlist_whitelist_allows_file() {
        let local = protocol_whitelist_for(r"C:\Users\me\Videos\stream.m3u8");
        assert!(local.split(',').any(|p| p == "file"));
    }

    #[test]
    fn media_input_path_uses_forward_slash_native_path() {
        let dir = std::env::temp_dir().join(format!("fc-hls-uri-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.m3u8");
        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"#EXTM3U\n")
            .unwrap();
        let uri = media_input_path(file.to_str().unwrap());
        assert!(!uri.starts_with("file:"));
        assert!(uri.contains('/'));
        assert!(uri.ends_with("test.m3u8"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
