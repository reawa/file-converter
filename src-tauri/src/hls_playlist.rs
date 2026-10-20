use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hls::{is_m3u8_url, media_input_path};
use crate::process_utils::hidden_command;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct HlsRange {
    pub start_secs: Option<f64>,
    pub end_secs: Option<f64>,
}

impl HlsRange {
    pub fn is_active(&self) -> bool {
        self.start_secs.is_some_and(|s| s > 0.0) || self.end_secs.is_some()
    }

    pub fn validate(&self) -> Result<(), String> {
        let start = self.start_secs.unwrap_or(0.0);
        if start < 0.0 {
            return Err("HLS start time cannot be negative.".to_string());
        }
        if let Some(end) = self.end_secs {
            if end <= 0.0 {
                return Err("HLS end time must be greater than zero.".to_string());
            }
            if end <= start {
                return Err("HLS end time must be after start time.".to_string());
            }
        }
        Ok(())
    }

    pub fn duration(&self) -> Option<f64> {
        match (self.start_secs.unwrap_or(0.0), self.end_secs) {
            (_, Some(end)) => Some((end - self.start_secs.unwrap_or(0.0)).max(0.0)),
            (start, None) if start > 0.0 => None,
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Segment {
    duration: f64,
    uri: String,
    map_uri: Option<String>,
}

/// Parse a time field: seconds, `MM:SS`, or `HH:MM:SS`.
pub fn parse_time_value(input: &str) -> Result<Option<f64>, String> {
    let s = input.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if let Ok(v) = s.parse::<f64>() {
        if v < 0.0 {
            return Err("Time cannot be negative.".to_string());
        }
        return Ok(Some(v));
    }
    let parts: Vec<&str> = s.split(':').collect();
    let secs = match parts.len() {
        1 => parts[0]
            .parse::<f64>()
            .map_err(|_| format!("Invalid time: {s}"))?,
        2 => {
            let m: f64 = parts[0]
                .parse()
                .map_err(|_| format!("Invalid time: {s}"))?;
            let sec: f64 = parts[1]
                .parse()
                .map_err(|_| format!("Invalid time: {s}"))?;
            m * 60.0 + sec
        }
        3 => {
            let h: f64 = parts[0]
                .parse()
                .map_err(|_| format!("Invalid time: {s}"))?;
            let m: f64 = parts[1]
                .parse()
                .map_err(|_| format!("Invalid time: {s}"))?;
            let sec: f64 = parts[2]
                .parse()
                .map_err(|_| format!("Invalid time: {s}"))?;
            h * 3600.0 + m * 60.0 + sec
        }
        _ => return Err(format!("Invalid time: {s}")),
    };
    Ok(Some(secs))
}

/// Extract `#t=start,end` or `?start=&end=` from an M3U8 URL.
pub fn parse_range_from_url(url: &str) -> HlsRange {
    let mut range = HlsRange::default();
    let (base, fragment) = match url.split_once('#') {
        Some(v) => v,
        None => (url, ""),
    };

    if !fragment.is_empty() {
        let frag = fragment.strip_prefix("t=").unwrap_or(fragment);
        let mut parts = frag.split(',');
        if let Some(start) = parts.next() {
            range.start_secs = parse_time_value(start).ok().flatten();
        }
        if let Some(end) = parts.next() {
            range.end_secs = parse_time_value(end).ok().flatten();
        }
    }

    if let Some(query) = base.split('?').nth(1) {
        for pair in query.split('&') {
            let Some((key, val)) = pair.split_once('=') else {
                continue;
            };
            match key.to_lowercase().as_str() {
                "start" | "t_start" | "start_secs" => {
                    range.start_secs = parse_time_value(val).ok().flatten();
                }
                "end" | "t_end" | "end_secs" => {
                    range.end_secs = parse_time_value(val).ok().flatten();
                }
                _ => {}
            }
        }
    }

    range
}

pub fn strip_url_time_params(url: &str) -> String {
    let mut out = url.trim().to_string();
    if let Some(idx) = out.find('#') {
        out.truncate(idx);
    }
    if let Some(idx) = out.find('?') {
        let base = &out[..idx];
        let query = &out[idx + 1..];
        let kept: Vec<&str> = query
            .split('&')
            .filter(|p| {
                let key = p.split('=').next().unwrap_or("").to_lowercase();
                !matches!(
                    key.as_str(),
                    "start" | "end" | "t_start" | "t_end" | "start_secs" | "end_secs"
                )
            })
            .collect();
        out = if kept.is_empty() {
            base.to_string()
        } else {
            format!("{}?{}", base, kept.join("&"))
        };
    }
    out
}

fn is_valid_m3u8_body(body: &str) -> bool {
    body.trim_start()
        .trim_start_matches('\u{feff}')
        .starts_with("#EXTM3U")
}

fn curl_program() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(r"C:\Windows\System32\curl.exe")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("curl")
    }
}

fn fetch_text(url: &str) -> Result<String, String> {
    let curl = curl_program();
    let curl_usable = !cfg!(windows) || curl.is_file();

    if curl_usable {
        let output = hidden_command(&curl)
            .args([
                "-sL",
                "--max-time",
                "120",
                "-A",
                "FileConverter/1.0",
                url,
            ])
            .output()
            .map_err(|e| format!("Failed to fetch playlist: {e}"))?;

        if output.status.success() && !output.stdout.is_empty() {
            let body = String::from_utf8_lossy(&output.stdout).into_owned();
            if is_valid_m3u8_body(&body) {
                return Ok(body);
            }
        }
    }

    #[cfg(windows)]
    {
        // Twitch/CDN playlists often use Content-Type binary/octet-stream; .Content is byte[]
        // and serializes as decimal lines — decode RawContentStream as UTF-8 instead.
        let ps = format!(
            "$r = Invoke-WebRequest -Uri '{}' -UseBasicParsing -TimeoutSec 120; [System.Text.Encoding]::UTF8.GetString($r.RawContentStream.ToArray())",
            url.replace('\'', "''")
        );
        let output = hidden_command("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .output()
            .map_err(|e| format!("Failed to fetch playlist: {e}"))?;

        if output.status.success() && !output.stdout.is_empty() {
            let body = String::from_utf8_lossy(&output.stdout).into_owned();
            if is_valid_m3u8_body(&body) {
                return Ok(body);
            }
        }
    }

    Err("Could not download a valid M3U8 playlist.".to_string())
}

fn load_playlist(source: &str) -> Result<(String, String), String> {
    if is_m3u8_url(source) {
        let url = strip_url_time_params(source);
        let body = fetch_text(&url)?;
        Ok((url, body))
    } else {
        let path = Path::new(source);
        let body = fs::read_to_string(path).map_err(|e| format!("Failed to read playlist: {e}"))?;
        let base = path
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();
        let base_url = if base.is_empty() {
            "file:///".to_string()
        } else {
            format!("file:///{}", base.replace('\\', "/"))
        };
        Ok((base_url, body))
    }
}

fn resolve_uri(base: &str, uri: &str) -> String {
    let uri = rewrite_twitch_unmuted_uri(uri.trim());
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri;
    }
    if is_m3u8_url(base) {
        if uri.starts_with('/') {
            if let Some(scheme_end) = base.find("://") {
                if let Some(host_start) = base[scheme_end + 3..].find('/') {
                    let root = &base[..scheme_end + 3 + host_start];
                    return format!("{root}{uri}");
                }
            }
        }
        let base_dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or(base);
        return format!("{base_dir}/{uri}");
    }
    let base_path = base.strip_prefix("file:///").unwrap_or(base);
    Path::new(base_path)
        .join(uri)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Twitch DVR playlists list `N-unmuted.ts` for segments whose audio was muted, but the
/// CDN only serves them as `N-muted.ts` (both `N-unmuted.ts` and `N.ts` return 403).
fn rewrite_twitch_unmuted_uri(uri: &str) -> String {
    const NEEDLE: &str = "-unmuted.";
    let Some(idx) = uri.rfind(NEEDLE) else {
        return uri.to_string();
    };
    let after = &uri[idx + NEEDLE.len()..];
    let ext_base = after.split(['?', '#']).next().unwrap_or(after);
    if !matches!(ext_base, "ts" | "mp4" | "m4s" | "aac") {
        return uri.to_string();
    }
    format!("{}-muted.{}", &uri[..idx], after)
}

fn is_master_playlist(content: &str) -> bool {
    content.contains("#EXT-X-STREAM-INF")
}

fn pick_variant_url(base: &str, content: &str) -> Result<String, String> {
    let mut best: Option<(u64, String)> = None;
    let mut pending_inf: Option<u64> = None;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("#EXT-X-STREAM-INF:") {
            pending_inf = line
                .split("BANDWIDTH=")
                .nth(1)
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.parse().ok());
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(bw) = pending_inf.take() {
                let url = resolve_uri(base, line);
                match &best {
                    Some((best_bw, _)) if bw <= *best_bw => {}
                    _ => best = Some((bw, url)),
                }
            }
        }
    }
    best.map(|(_, u)| u)
        .ok_or_else(|| "No variant streams found in master playlist.".to_string())
}

fn parse_media_playlist(base: &str, content: &str) -> Result<Vec<Segment>, String> {
    let mut segments = Vec::new();
    let mut pending_duration: Option<f64> = None;
    let mut pending_map: Option<String> = None;
    let mut target_duration: u64 = 10;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("#EXT-X-TARGETDURATION:") {
            target_duration = line
                .strip_prefix("#EXT-X-TARGETDURATION:")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10);
        } else if line.starts_with("#EXT-X-MAP:") {
            if let Some(uri) = extract_attr(line, "URI") {
                pending_map = Some(resolve_uri(base, uri.trim_matches(|c| c == '"')));
            }
        } else if line.starts_with("#EXTINF:") {
            let dur_str = line
                .strip_prefix("#EXTINF:")
                .and_then(|s| s.split(',').next())
                .unwrap_or("0");
            pending_duration = Some(dur_str.parse().unwrap_or(0.0));
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(duration) = pending_duration.take() {
                segments.push(Segment {
                    duration,
                    uri: resolve_uri(base, line),
                    map_uri: pending_map.clone(),
                });
            }
        }
    }

    if segments.is_empty() {
        return Err("Playlist contains no media segments.".to_string());
    }

    let _ = target_duration;
    Ok(segments)
}

fn extract_attr(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let idx = line.find(&needle)?;
    let rest = &line[idx + needle.len()..];
    let end = rest.find(',').unwrap_or(rest.len());
    Some(rest[..end].trim().trim_matches('"').to_string())
}

fn select_segments(segments: &[Segment], range: &HlsRange) -> Result<Vec<Segment>, String> {
    let start = range.start_secs.unwrap_or(0.0);
    let end = range.end_secs.unwrap_or(f64::MAX);
    let mut elapsed = 0.0;
    let mut selected = Vec::new();

    for seg in segments {
        let seg_start = elapsed;
        let seg_end = elapsed + seg.duration;
        if seg_end > start && seg_start < end {
            selected.push(seg.clone());
        }
        elapsed = seg_end;
        if seg_start >= end {
            break;
        }
    }

    if selected.is_empty() {
        return Err(format!(
            "No segments overlap {start:.1}s–{}.",
            if range.end_secs.is_some() {
                format!("{end:.1}s")
            } else {
                "end".to_string()
            }
        ));
    }
    Ok(selected)
}

fn write_trimmed_playlist(path: &Path, segments: &[Segment], version: &str) -> Result<(), String> {
    let max_dur = segments
        .iter()
        .map(|s| s.duration.ceil() as u64)
        .max()
        .unwrap_or(10);

    let mut lines = vec![
        "#EXTM3U".to_string(),
        format!("#EXT-X-VERSION:{version}"),
        format!("#EXT-X-TARGETDURATION:{max_dur}"),
        "#EXT-X-MEDIA-SEQUENCE:0".to_string(),
        "#EXT-X-PLAYLIST-TYPE:VOD".to_string(),
    ];

    let mut last_map: Option<String> = None;
    for seg in segments {
        if seg.map_uri.as_ref() != last_map.as_ref() {
            if let Some(map) = &seg.map_uri {
                lines.push(format!("#EXT-X-MAP:URI=\"{map}\""));
                last_map = Some(map.clone());
            }
        }
        lines.push(format!("#EXTINF:{:.3},", seg.duration));
        lines.push(seg.uri.clone());
    }
    lines.push("#EXT-X-ENDLIST".to_string());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create temp dir: {e}"))?;
    }
    fs::write(path, lines.join("\n") + "\n")
        .map_err(|e| format!("Failed to write trimmed playlist: {e}"))
}

/// Load the media playlist for `source`, following a master playlist to its best variant.
/// Returns (base URL for resolving segment URIs, media playlist body).
fn load_media_playlist(source: &str) -> Result<(String, String), String> {
    let (base, content) = load_playlist(source)?;
    if is_master_playlist(&content) {
        let variant = pick_variant_url(&base, &content)?;
        let body = fetch_text(&variant)?;
        Ok((variant, body))
    } else {
        Ok((base, content))
    }
}

fn playlist_version(content: &str) -> &str {
    content
        .lines()
        .find(|l| l.starts_with("#EXT-X-VERSION:"))
        .and_then(|l| l.strip_prefix("#EXT-X-VERSION:"))
        .unwrap_or("3")
}

fn temp_playlist_path() -> PathBuf {
    std::env::temp_dir()
        .join("file-converter-hls")
        .join(format!("range-{}.m3u8", uuid::Uuid::new_v4()))
}

/// Build a local playlist containing only segments in the requested time range.
pub fn prepare_ranged_playlist(source: &str, range: &HlsRange) -> Result<PathBuf, String> {
    range.validate()?;
    if !range.is_active() {
        return Err("HLS range is not active.".to_string());
    }

    let (base, media_content) = load_media_playlist(source)?;
    let segments = parse_media_playlist(&base, &media_content)?;
    let selected = select_segments(&segments, range)?;

    let playlist_path = temp_playlist_path();
    write_trimmed_playlist(&playlist_path, &selected, playlist_version(&media_content))?;

    Ok(playlist_path)
}

/// Full (unranged) download of a playlist that lists Twitch `-unmuted` segments: those URIs
/// 403 on the CDN, so write a local copy with rewritten URIs. Returns None when the playlist
/// needs no rewrite, so ordinary HLS sources keep going straight to FFmpeg.
fn prepare_rewritten_full_playlist(source: &str) -> Result<Option<(PathBuf, f64)>, String> {
    let (base, media_content) = load_media_playlist(source)?;
    if !media_content.contains("-unmuted.") {
        return Ok(None);
    }
    let segments = parse_media_playlist(&base, &media_content)?;
    let total: f64 = segments.iter().map(|s| s.duration).sum();
    let playlist_path = temp_playlist_path();
    write_trimmed_playlist(&playlist_path, &segments, playlist_version(&media_content))?;
    Ok(Some((playlist_path, total)))
}

/// Resolve ffmpeg input for HLS sources; returns (input_path, temp_playlist, effective_duration).
pub fn resolve_hls_input(
    source: &str,
    range: &HlsRange,
) -> Result<(String, Option<PathBuf>, Option<f64>), String> {
    if !crate::hls::is_hls_source(source, &crate::hls::effective_input_ext(source)) {
        return Ok((source.to_string(), None, None));
    }

    if range.is_active() {
        range.validate()?;
        let playlist = prepare_ranged_playlist(source, range)?;
        let path_str = media_input_path(&playlist.to_string_lossy());
        let duration = range.duration().or_else(|| {
            let start = range.start_secs.unwrap_or(0.0);
            range.end_secs.map(|e| e - start)
        });
        return Ok((path_str, Some(playlist), duration));
    }

    if let Some((playlist, total)) = prepare_rewritten_full_playlist(source).ok().flatten() {
        let path_str = media_input_path(&playlist.to_string_lossy());
        return Ok((path_str, Some(playlist), Some(total)));
    }

    Ok((source.to_string(), None, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hhmmss() {
        assert_eq!(parse_time_value("6:00:00").unwrap(), Some(21600.0));
        assert_eq!(parse_time_value("90").unwrap(), Some(90.0));
        assert_eq!(parse_time_value("").unwrap(), None);
    }

    #[test]
    fn parses_url_fragment_range() {
        let r = parse_range_from_url("https://x.com/a.m3u8#t=3600,7200");
        assert_eq!(r.start_secs, Some(3600.0));
        assert_eq!(r.end_secs, Some(7200.0));
    }

    #[test]
    fn extracts_map_uri_from_twitch_tag() {
        assert_eq!(
            extract_attr(r#"#EXT-X-MAP:URI="init-0.mp4""#, "URI").as_deref(),
            Some("init-0.mp4")
        );
        assert_eq!(
            extract_attr(r#"#EXT-X-MAP:URI="init.mp4",BYTERANGE=123@0"#, "URI").as_deref(),
            Some("init.mp4")
        );
    }

    #[test]
    fn rejects_byte_decimal_garbage_as_playlist() {
        assert!(!is_valid_m3u8_body("35\n69\n88\n84\n77\n"));
        assert!(is_valid_m3u8_body("#EXTM3U\n#EXT-X-VERSION:3\n"));
    }

    /// Network smoke test: cargo test hls_range_smoke -- --ignored --nocapture
    /// Window 5:03:00-5:05:00 of this VOD is Twitch-muted (playlist lists -unmuted).
    #[test]
    #[ignore]
    fn hls_range_smoke() {
        use crate::process_utils::hidden_command;
        use crate::hls::PROTOCOL_WHITELIST;

        let url = "https://d2vi6trrdongqn.cloudfront.net/577fbb474357ba645435_shinyatheninja_320129887838_1788783599/chunked/index-dvr.m3u8";
        let range = HlsRange {
            start_secs: Some(18180.0),
            end_secs: Some(18300.0),
        };
        let playlist = prepare_ranged_playlist(url, &range).expect("prepare playlist");
        let content = fs::read_to_string(&playlist).expect("read trimmed");
        assert!(content.contains("1818-muted.ts"), "{}", &content[..content.len().min(800)]);
        assert!(!content.contains("-unmuted."));

        let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/win64");
        let input = media_input_path(&playlist.to_string_lossy());
        let out = std::env::temp_dir().join(format!("fc-hls-smoke-{}.mp4", uuid::Uuid::new_v4()));
        let status = hidden_command(bin.join("ffmpeg.exe"))
            .args([
                "-y", "-protocol_whitelist", PROTOCOL_WHITELIST, "-fflags", "+genpts",
                "-i", &input, "-c", "copy", "-avoid_negative_ts", "make_zero",
                "-bsf:a", "aac_adtstoasc", "-movflags", "+faststart",
                out.to_str().unwrap(),
            ])
            .status()
            .expect("ffmpeg");
        let _ = fs::remove_file(&playlist);
        assert!(status.success(), "ffmpeg failed on trimmed HLS playlist");

        let probe = hidden_command(bin.join("ffprobe.exe"))
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nokey=1:noprint_wrappers=1", out.to_str().unwrap()])
            .output()
            .expect("ffprobe");
        let secs: f64 = String::from_utf8_lossy(&probe.stdout).trim().parse().unwrap_or(0.0);
        println!("smoke output: {} ({secs:.1}s)", out.display());
        assert!(secs >= 115.0, "output too short: {secs}s");
    }

    #[test]
    fn selects_segment_window() {
        let segments = vec![
            Segment {
                duration: 10.0,
                uri: "a.ts".into(),
                map_uri: None,
            },
            Segment {
                duration: 10.0,
                uri: "b.ts".into(),
                map_uri: None,
            },
            Segment {
                duration: 10.0,
                uri: "c.ts".into(),
                map_uri: None,
            },
        ];
        let range = HlsRange {
            start_secs: Some(5.0),
            end_secs: Some(25.0),
        };
        let picked = select_segments(&segments, &range).unwrap();
        assert_eq!(picked.len(), 3);
    }

    #[test]
    fn rewrites_twitch_unmuted_segment_names() {
        assert_eq!(rewrite_twitch_unmuted_uri("1818-unmuted.ts"), "1818-muted.ts");
        assert_eq!(rewrite_twitch_unmuted_uri("12-unmuted.mp4"), "12-muted.mp4");
        assert_eq!(
            rewrite_twitch_unmuted_uri("9-unmuted.ts?sig=abc"),
            "9-muted.ts?sig=abc"
        );
        assert_eq!(rewrite_twitch_unmuted_uri("1800.ts"), "1800.ts");
        assert_eq!(
            rewrite_twitch_unmuted_uri("https://cdn.example/chunked/50-unmuted.ts"),
            "https://cdn.example/chunked/50-muted.ts"
        );
    }

    #[test]
    fn full_playlist_rewritten_only_when_unmuted_present() {
        let dir = std::env::temp_dir().join(format!("fc-hls-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let twitch = dir.join("twitch.m3u8");
        fs::write(
            &twitch,
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXTINF:10.000,\n0.ts\n#EXTINF:10.000,\n1-unmuted.ts\n#EXTINF:5.000,\n2.ts\n#EXT-X-ENDLIST\n",
        )
        .unwrap();
        let plain = dir.join("plain.m3u8");
        fs::write(
            &plain,
            "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10.000,\n0.ts\n#EXT-X-ENDLIST\n",
        )
        .unwrap();

        let (out, total) = prepare_rewritten_full_playlist(&twitch.to_string_lossy())
            .unwrap()
            .expect("twitch playlist should be rewritten");
        let body = fs::read_to_string(&out).unwrap();
        assert!((total - 25.0).abs() < 1e-6);
        assert!(body.contains("1-muted.ts"));
        assert!(!body.contains("-unmuted."));
        assert_eq!(body.matches("#EXTINF").count(), 3);
        assert!(body.contains("#EXT-X-ENDLIST"));

        assert!(prepare_rewritten_full_playlist(&plain.to_string_lossy())
            .unwrap()
            .is_none());

        let _ = fs::remove_file(out);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_playlist_rewrites_unmuted_uris() {
        let content = r#"#EXTM3U
#EXT-X-TARGETDURATION:10
#EXTINF:10.000,
1818-unmuted.ts
#EXTINF:10.000,
1819.ts
#EXTINF:10.000,
1820-unmuted.mp4
"#;
        let segs = parse_media_playlist(
            "https://cdn.example.com/vod/chunked/index-dvr.m3u8",
            content,
        )
        .unwrap();
        assert_eq!(segs.len(), 3);
        assert!(segs[0].uri.ends_with("/1818-muted.ts"));
        assert!(segs[1].uri.ends_with("/1819.ts"));
        assert!(segs[2].uri.ends_with("/1820-muted.mp4"));
        assert!(!segs.iter().any(|s| s.uri.contains("-unmuted.")));
    }

    #[test]
    fn select_five_hour_window_count() {
        let segments: Vec<Segment> = (0..3000)
            .map(|i| Segment {
                duration: 10.0,
                uri: format!("{i}.ts"),
                map_uri: None,
            })
            .collect();
        let range = HlsRange {
            start_secs: Some(18_000.0),
            end_secs: Some(21_000.0),
        };
        let picked = select_segments(&segments, &range).unwrap();
        assert_eq!(picked.len(), 300);
        assert_eq!(picked[0].uri, "1800.ts");
        assert_eq!(picked.last().unwrap().uri, "2099.ts");
    }
}
