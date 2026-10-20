use std::cell::RefCell;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use regex::Regex;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::converter::{
    avif_output_allowed, build_ffmpeg_args, get_hw_encoders_blocking, looks_like_hw_encoder_failure,
};
use crate::format_registry::{normalize_ext, ConversionTier, MediaCategory};
use crate::hls::{effective_input_ext, is_m3u8_url, output_stem_from_input};
use crate::hls_playlist::{resolve_hls_input, HlsRange};
use crate::path_validation::{default_downloads_dir, validate_output_path_contained};
use crate::probe::{decide_tier, probe_file};
use crate::process_utils::{hidden_background_command, hidden_command};
use crate::youtube::{self, is_ytdlp_url};

static DURATION_RE: OnceLock<Regex> = OnceLock::new();
static TIME_RE: OnceLock<Regex> = OnceLock::new();

fn duration_re() -> &'static Regex {
    DURATION_RE.get_or_init(|| {
        Regex::new(r"Duration:\s*(\d+):(\d+):(\d+(?:\.\d+)?)").expect("valid duration regex")
    })
}

fn time_re() -> &'static Regex {
    TIME_RE.get_or_init(|| {
        Regex::new(r"time=(\d+):(\d+):(\d+(?:\.\d+)?)").expect("valid time regex")
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversionJob {
    pub id: String,
    pub path: String,
    pub output_format: String,
    #[serde(default)]
    pub hls_start_secs: Option<f64>,
    #[serde(default)]
    pub hls_end_secs: Option<f64>,
    #[serde(default)]
    pub youtube_quality: Option<String>,
    /// Optional output file stem (no extension). When unset, derived from the source.
    #[serde(default)]
    pub output_name: Option<String>,
}

impl ConversionJob {
    pub fn hls_range(&self) -> HlsRange {
        HlsRange {
            start_secs: self.hls_start_secs,
            end_secs: self.hls_end_secs,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgressEvent {
    pub id: String,
    pub progress: f64,
    pub status: String,
    pub elapsed_secs: f64,
    pub eta_secs: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompleteEvent {
    pub id: String,
    pub output_path: String,
    pub elapsed_secs: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorEvent {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CancelledEvent {
    pub id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkippedEvent {
    pub id: String,
    pub message: String,
}

/// Receiver for queue events: the Tauri window or the CLI.
pub trait JobSink: Send + Sync {
    fn progress(&self, event: ProgressEvent);
    fn complete(&self, event: CompleteEvent);
    fn error(&self, event: ErrorEvent);
    fn cancelled(&self, event: CancelledEvent);
    fn skipped(&self, event: SkippedEvent);
    fn queue_complete(&self);
}

impl JobSink for AppHandle {
    fn progress(&self, event: ProgressEvent) {
        let _ = self.emit("conversion-progress", event);
    }
    fn complete(&self, event: CompleteEvent) {
        let _ = self.emit("conversion-complete", event);
    }
    fn error(&self, event: ErrorEvent) {
        let _ = self.emit("conversion-error", event);
    }
    fn cancelled(&self, event: CancelledEvent) {
        let _ = self.emit("conversion-cancelled", event);
    }
    fn skipped(&self, event: SkippedEvent) {
        let _ = self.emit("conversion-skipped", event);
    }
    fn queue_complete(&self) {
        let _ = self.emit("queue-complete", ());
    }
}

struct ProgressTracker {
    last_emit: Option<Instant>,
    last_progress: f64,
    /// (time, progress) the current ETA rate is measured from; reset when progress goes
    /// backwards (yt-dlp downloads video and audio streams separately).
    rate_origin: Option<(Instant, f64)>,
}

/// Per-job event emitter that stamps elapsed time and ETA onto progress events.
struct JobReporter<'a> {
    sink: &'a dyn JobSink,
    id: String,
    started: Instant,
    tracker: RefCell<ProgressTracker>,
}

impl<'a> JobReporter<'a> {
    fn new(sink: &'a dyn JobSink, id: &str) -> Self {
        Self {
            sink,
            id: id.to_string(),
            started: Instant::now(),
            tracker: RefCell::new(ProgressTracker {
                last_emit: None,
                last_progress: 0.0,
                rate_origin: None,
            }),
        }
    }

    fn last_progress(&self) -> f64 {
        self.tracker.borrow().last_progress
    }

    fn elapsed_secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Throttled: emits at most every 200 ms unless progress moved by 1% or more.
    fn progress(&self, progress: f64, status: &str) {
        let now = Instant::now();
        let due = {
            let t = self.tracker.borrow();
            progress >= 100.0
                || (progress - t.last_progress).abs() >= 1.0
                || t
                    .last_emit
                    .map_or(true, |at| now.duration_since(at) >= Duration::from_millis(200))
        };
        if due {
            self.emit_progress(progress, status, now);
        }
    }

    fn progress_now(&self, progress: f64, status: impl Into<String>) {
        self.emit_progress(progress, &status.into(), Instant::now());
    }

    fn emit_progress(&self, progress: f64, status: &str, now: Instant) {
        let eta_secs = {
            let mut t = self.tracker.borrow_mut();
            t.last_emit = Some(now);
            t.last_progress = progress;
            estimate_eta(&mut t.rate_origin, progress, now)
        };
        self.sink.progress(ProgressEvent {
            id: self.id.clone(),
            progress,
            status: status.to_string(),
            elapsed_secs: self.elapsed_secs(),
            eta_secs,
        });
    }

    fn complete(&self, out_path: &Path) {
        self.progress_now(100.0, "Complete");
        self.sink.complete(CompleteEvent {
            id: self.id.clone(),
            output_path: out_path.to_string_lossy().to_string(),
            elapsed_secs: self.elapsed_secs(),
        });
    }

    fn error(&self, message: impl Into<String>) {
        self.sink.error(ErrorEvent {
            id: self.id.clone(),
            message: message.into(),
        });
    }

    fn cancelled(&self) {
        self.sink.cancelled(CancelledEvent {
            id: self.id.clone(),
        });
    }
}

/// Remaining seconds from the observed progress rate. Needs at least 0.5% of movement
/// since `origin` before it returns an estimate.
fn estimate_eta(origin: &mut Option<(Instant, f64)>, progress: f64, now: Instant) -> Option<f64> {
    if !(progress > 0.0 && progress < 100.0) {
        return None;
    }
    match *origin {
        Some((t0, p0)) if progress >= p0 => {
            let moved = progress - p0;
            if moved < 0.5 {
                return None;
            }
            let secs = now.duration_since(t0).as_secs_f64();
            Some(secs / moved * (100.0 - progress))
        }
        _ => {
            *origin = Some((now, progress));
            None
        }
    }
}

pub struct QueueState {
    pub cancel: AtomicBool,
    pub running: AtomicBool,
    pub child: Mutex<Option<Child>>,
}

impl QueueState {
    pub fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
            running: AtomicBool::new(false),
            child: Mutex::new(None),
        }
    }
}

/// Replace characters that are invalid or problematic in Windows output paths.
fn sanitize_output_stem(stem: &str) -> String {
    let cleaned: String = stem
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0' => '_',
            '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}' => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn default_stem_for_input(input: &str) -> String {
    if is_ytdlp_url(input) {
        sanitize_output_stem(&youtube::output_stem_for_url(input))
    } else {
        sanitize_output_stem(&output_stem_from_input(input))
    }
}

/// Resolve output directory: explicit folder, else Downloads for remote/HLS URLs,
/// else the local file's parent ("home") directory.
pub fn resolve_output_dir(input: &str, output_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = output_dir.filter(|d| d.as_os_str().len() > 0) {
        return dir.to_path_buf();
    }
    if is_m3u8_url(input) || is_ytdlp_url(input) {
        return default_downloads_dir();
    }
    Path::new(input)
        .parent()
        .filter(|p| p.as_os_str().len() > 0)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_downloads_dir)
}

pub fn output_path(
    input: &str,
    output_dir: Option<&Path>,
    output_ext: &str,
    output_name: Option<&str>,
) -> PathBuf {
    let stem = match output_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(custom) => sanitize_output_stem(custom),
        None => default_stem_for_input(input),
    };
    let dir = resolve_output_dir(input, output_dir);
    dir.join(format!("{}.{}", stem, normalize_ext(output_ext)))
}

pub fn ensure_output_parent(out_path: &Path) -> Result<(), String> {
    let parent = out_path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| format!("Failed to create output directory: {e}"))
}

fn parse_duration(text: &str) -> Option<f64> {
    let caps = duration_re().captures(text)?;
    let h = caps.get(1)?.as_str().parse::<f64>().unwrap_or(0.0);
    let m = caps.get(2)?.as_str().parse::<f64>().unwrap_or(0.0);
    let s = caps.get(3)?.as_str().parse::<f64>().unwrap_or(0.0);
    Some(h * 3600.0 + m * 60.0 + s)
}

fn parse_time(text: &str) -> Option<f64> {
    let caps = time_re().captures(text)?;
    let h = caps.get(1)?.as_str().parse::<f64>().unwrap_or(0.0);
    let m = caps.get(2)?.as_str().parse::<f64>().unwrap_or(0.0);
    let s = caps.get(3)?.as_str().parse::<f64>().unwrap_or(0.0);
    Some(h * 3600.0 + m * 60.0 + s)
}

fn run_ffmpeg(
    rep: &JobReporter,
    program: &str,
    ffmpeg_args: Vec<String>,
    tier_label: &str,
    total_duration: Option<f64>,
    state: &Arc<QueueState>,
) -> (bool, Vec<String>) {
    let mut child = match hidden_background_command(program)
        .args(&ffmpeg_args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            rep.error(format!("Failed to start ffmpeg: {e}"));
            return (false, vec![]);
        }
    };

    let stderr = child.stderr.take();
    if let Ok(mut guard) = state.child.lock() {
        *guard = Some(child);
    }

    let mut duration = total_duration;
    let mut stderr_tail: Vec<String> = Vec::new();

    if let Some(stderr) = stderr {
        for line in CrLfLines::new(stderr) {
            if state.cancel.load(Ordering::SeqCst) {
                break;
            }

            let is_stats = line.starts_with("frame=") || line.starts_with("size=");
            if !is_stats && !line.trim().is_empty() {
                stderr_tail.push(line.clone());
                if stderr_tail.len() > 12 {
                    stderr_tail.remove(0);
                }
            }

            if duration.is_none() {
                if let Some(d) = parse_duration(&line) {
                    duration = Some(d);
                }
            }

            if let (Some(dur), Some(cur)) = (duration, parse_time(&line)) {
                if dur > 0.0 {
                    let p = (cur / dur * 100.0).min(99.0);
                    rep.progress(p, tier_label);
                }
            }
        }
    }

    if state.cancel.load(Ordering::SeqCst) {
        if let Ok(mut guard) = state.child.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        return (false, stderr_tail);
    }

    let status = if let Ok(mut guard) = state.child.lock() {
        match guard.take() {
            Some(mut c) => c.wait(),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "ffmpeg process missing",
            )),
        }
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "failed to lock queue",
        ))
    };

    match status {
        Ok(s) if s.success() => (true, stderr_tail),
        Ok(_) => (false, stderr_tail),
        Err(e) => {
            rep.error(format!("Failed waiting for ffmpeg: {e}"));
            (false, stderr_tail)
        }
    }
}

/// Line iterator that splits on `\r` as well as `\n`. FFmpeg ends its periodic stats line
/// with `\r`, so `BufRead::lines` would deliver all progress in one burst at exit.
struct CrLfLines<R: Read> {
    reader: BufReader<R>,
    done: bool,
}

impl<R: Read> CrLfLines<R> {
    fn new(inner: R) -> Self {
        Self {
            reader: BufReader::new(inner),
            done: false,
        }
    }
}

impl<R: Read> Iterator for CrLfLines<R> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        let mut line = Vec::new();
        while !self.done {
            let buf = match self.reader.fill_buf() {
                Ok(buf) => buf,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.done = true;
                    break;
                }
            };
            if buf.is_empty() {
                self.done = true;
                break;
            }
            match buf.iter().position(|b| *b == b'\n' || *b == b'\r') {
                Some(i) => {
                    line.extend_from_slice(&buf[..i]);
                    self.reader.consume(i + 1);
                    if line.is_empty() {
                        // Second half of a `\r\n` pair, or a blank line.
                        continue;
                    }
                    return Some(String::from_utf8_lossy(&line).into_owned());
                }
                None => {
                    let n = buf.len();
                    line.extend_from_slice(buf);
                    self.reader.consume(n);
                }
            }
        }
        (!line.is_empty()).then(|| String::from_utf8_lossy(&line).into_owned())
    }
}

fn parse_ytdlp_percent(line: &str) -> Option<f64> {
    let idx = line.find('%')?;
    let start = line[..idx].rfind(|c: char| !c.is_ascii_digit() && c != '.')? + 1;
    line[start..idx].parse().ok()
}

fn probe_output_duration(path: &Path) -> Result<f64, String> {
    let output = hidden_command(crate::ffmpeg_tools::ffprobe_path())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
            &path.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("ffprobe failed: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("Invalid duration from ffprobe: {}", text.trim()))?;
    if !value.is_finite() || value < 0.0 {
        return Err("Invalid duration from ffprobe.".to_string());
    }
    Ok(value)
}

fn process_youtube_job(
    rep: &JobReporter,
    job: &ConversionJob,
    output_dir_path: Option<&PathBuf>,
    state: &Arc<QueueState>,
) {
    let out_path = output_path(
        &job.path,
        output_dir_path.map(|p| p.as_path()),
        &job.output_format,
        job.output_name.as_deref(),
    );
    let output_root = resolve_output_dir(&job.path, output_dir_path.map(|p| p.as_path()));

    if let Err(e) = validate_output_path_contained(&out_path, &output_root) {
        rep.error(e);
        return;
    }
    if let Err(e) = ensure_output_parent(&out_path) {
        rep.error(e);
        return;
    }

    let quality = job.youtube_quality.as_deref().unwrap_or("best");
    let args = match youtube::build_download_args(&job.path, quality, &out_path) {
        Ok(a) => a,
        Err(e) => {
            rep.error(e);
            return;
        }
    };

    rep.progress_now(0.0, "Downloading…");

    let mut child = match hidden_command(crate::ytdlp_tools::ytdlp_path())
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            rep.error(format!("Failed to start yt-dlp: {e}"));
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    if let Ok(mut guard) = state.child.lock() {
        *guard = Some(child);
    }

    let mut err_tail: Vec<String> = Vec::new();

    let mut handle_line = |line: &str| {
        if let Some(p) = parse_ytdlp_percent(line) {
            rep.progress(p.min(99.0), "Downloading…");
        } else if line.contains("[ExtractAudio]") || line.contains("[Merger]") {
            rep.progress(rep.last_progress().max(90.0), "Processing…");
        }
        if line.contains("ERROR") || line.starts_with("ERROR:") {
            err_tail.push(line.to_string());
            if err_tail.len() > 8 {
                err_tail.remove(0);
            }
        }
    };

    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if state.cancel.load(Ordering::SeqCst) {
                break;
            }
            handle_line(&line);
        }
    }
    if let Some(stderr) = stderr {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if state.cancel.load(Ordering::SeqCst) {
                break;
            }
            handle_line(&line);
        }
    }

    if state.cancel.load(Ordering::SeqCst) {
        if let Ok(mut guard) = state.child.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        rep.cancelled();
        let _ = fs::remove_file(&out_path);
        return;
    }

    let status = if let Ok(mut guard) = state.child.lock() {
        match guard.take() {
            Some(mut c) => c.wait(),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "yt-dlp process missing",
            )),
        }
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "failed to lock queue",
        ))
    };

    match status {
        Ok(s) if s.success() => match youtube::finalize_download_output(&out_path) {
            Ok(()) => {
                rep.complete(&out_path);
            }
            Err(e) => {
                rep.error(e);
            }
        },
        Ok(_) => {
            let detail = err_tail
                .last()
                .cloned()
                .unwrap_or_else(|| "yt-dlp download failed.".to_string());
            rep.error(detail);
        }
        Err(e) => {
            rep.error(format!("Failed waiting for yt-dlp: {e}"));
        }
    }
}

fn process_job(
    sink: &dyn JobSink,
    job: &ConversionJob,
    output_dir_path: Option<&PathBuf>,
    state: &Arc<QueueState>,
    hw: &crate::converter::HwEncoders,
) {
    let rep = JobReporter::new(sink, &job.id);
    if is_ytdlp_url(&job.path) {
        process_youtube_job(&rep, job, output_dir_path, state);
        return;
    }

    rep.progress_now(0.0, "Preparing…");

    let input_ext = effective_input_ext(&job.path);

    let hls_range = job.hls_range();
    let (ffmpeg_input, hls_temp, range_duration) = match resolve_hls_input(&job.path, &hls_range) {
        Ok(v) => v,
        Err(e) => {
            rep.error(e);
            return;
        }
    };

    let probe_result = match probe_file(&job.path) {
        Ok(p) => p,
        Err(e) => {
            rep.error(e);
            return;
        }
    };

    let tier = decide_tier(&probe_result, &input_ext, &job.output_format);
    let out_path = output_path(
        &job.path,
        output_dir_path.map(|p| p.as_path()),
        &job.output_format,
        job.output_name.as_deref(),
    );

    let output_root = resolve_output_dir(&job.path, output_dir_path.map(|p| p.as_path()));

    if let Err(e) = validate_output_path_contained(&out_path, &output_root) {
        rep.error(e);
        return;
    }

    if let Err(e) = ensure_output_parent(&out_path) {
        rep.error(e);
        return;
    }

    if normalize_ext(&job.output_format) == "avif" && !avif_output_allowed() {
        rep.error(
            "AVIF output requires an AV1 encoder (libsvtav1 or libaom-av1) in your FFmpeg build.",
        );
        return;
    }

    let tier_label = match tier {
        ConversionTier::Copy => "Stream copy — no quality loss",
        ConversionTier::EncodeHq => "High-quality encode",
    };

  // Fast image copy
    if tier == ConversionTier::Copy
        && probe_result.category == MediaCategory::Image
        && normalize_ext(&input_ext) == normalize_ext(&job.output_format)
    {
        let input_path = PathBuf::from(&job.path);
        rep.progress_now(0.0, tier_label);
        match fs::copy(&input_path, &out_path) {
            Ok(_) => {
                rep.complete(&out_path);
            }
            Err(e) => {
                rep.error(format!("Failed to copy file: {e}"));
            }
        }
        return;
    }

    rep.progress_now(0.0, tier_label);

    let is_fast_job = probe_result.category == MediaCategory::Image
        || (probe_result.duration_secs.is_none()
            && probe_result.category == MediaCategory::Audio);

    let progress_duration = range_duration.or(probe_result.duration_secs);

    let mut cpu_only = false;
    let mut success = false;
    let mut stderr_tail: Vec<String> = Vec::new();

    for attempt in 0..2 {
        if state.cancel.load(Ordering::SeqCst) {
            break;
        }

        let ffmpeg_args = build_ffmpeg_args(
            &ffmpeg_input,
            &out_path.to_string_lossy(),
            &probe_result,
            &input_ext,
            &job.output_format,
            tier,
            hw,
            crate::converter::get_cpu_encoders(),
            cpu_only,
        );
        let program = ffmpeg_args[0].clone();
        let args: Vec<String> = ffmpeg_args.into_iter().skip(1).collect();

        let (ok, tail) = run_ffmpeg(
            &rep,
            &program,
            args,
            tier_label,
            progress_duration,
            state,
        );
        stderr_tail = tail;

        if state.cancel.load(Ordering::SeqCst) {
            break;
        }

        if ok {
            success = true;
            break;
        }

        if !cpu_only && attempt == 0 && looks_like_hw_encoder_failure(&stderr_tail) {
            cpu_only = true;
            continue;
        }

        break;
    }

    if state.cancel.load(Ordering::SeqCst) {
        let _ = fs::remove_file(&out_path);
        rep.cancelled();
        if let Some(temp) = hls_temp {
            let _ = fs::remove_file(temp);
        }
        return;
    }

    if success {
        if let Some(expected) = range_duration.filter(|d| *d > 1.0) {
            match probe_output_duration(&out_path) {
                Ok(actual) if actual + 0.5 < expected * 0.85 => {
                    let _ = fs::remove_file(&out_path);
                    rep.error(format!(
                        "Output too short ({:.0}s) for requested range ({:.0}s). Some stream segments may be missing from the CDN.",
                        actual, expected
                    ));
                    if let Some(temp) = hls_temp {
                        let _ = fs::remove_file(temp);
                    }
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    rep.error(format!("Could not verify output duration: {e}"));
                    if let Some(temp) = hls_temp {
                        let _ = fs::remove_file(temp);
                    }
                    return;
                }
            }
        }
        rep.complete(&out_path);
        if let Some(temp) = hls_temp {
            let _ = fs::remove_file(temp);
        }
        return;
    }

    if is_fast_job && stderr_tail.is_empty() {
        // no stderr captured but failed — still report
    }

    // FFmpeg ends most failures with a generic "Conversion failed!"; report the cause instead.
    let detail = stderr_tail
        .iter()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && *l != "Conversion failed!")
        .map(str::to_string)
        .unwrap_or_else(|| "FFmpeg exited with an error.".to_string());
    let _ = fs::remove_file(&out_path);
    rep.error(detail);

    if let Some(temp) = hls_temp {
        let _ = fs::remove_file(temp);
    }
}

/// Run jobs one at a time on a background thread. Returns None (after emitting an error)
/// when a queue is already running.
pub fn run_queue(
    sink: Arc<dyn JobSink>,
    jobs: Vec<ConversionJob>,
    output_dir: Option<String>,
    state: Arc<QueueState>,
) -> Option<thread::JoinHandle<()>> {
    if state.running.swap(true, Ordering::SeqCst) {
        sink.error(ErrorEvent {
            id: "".to_string(),
            message: "A conversion is already running.".to_string(),
        });
        return None;
    }

    state.cancel.store(false, Ordering::SeqCst);
    let output_dir_path = output_dir.map(PathBuf::from);
    Some(thread::spawn(move || {
        let hw = get_hw_encoders_blocking();
        for job in jobs {
            if state.cancel.load(Ordering::SeqCst) {
                sink.skipped(SkippedEvent {
                    id: job.id.clone(),
                    message: "Skipped — conversion cancelled".to_string(),
                });
                continue;
            }

            process_job(
                sink.as_ref(),
                &job,
                output_dir_path.as_ref(),
                &state,
                hw,
            );

            thread::sleep(Duration::from_millis(50));
        }

        state.running.store(false, Ordering::SeqCst);
        sink.queue_complete();
    }))
}

pub fn cancel(state: &QueueState) {
    state.cancel.store(true, Ordering::SeqCst);
    if let Ok(mut guard) = state.child.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            // Wait so the process releases the output file before the job deletes it.
            let _ = child.wait();
        }
    }
}

pub fn new_job_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn resolve_output_paths(
    jobs: &[ConversionJob],
    output_dir: Option<&Path>,
) -> Vec<(String, PathBuf)> {
    jobs.iter()
        .map(|job| {
            let out = output_path(
                &job.path,
                output_dir,
                &job.output_format,
                job.output_name.as_deref(),
            );
            (job.id.clone(), out)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn splits_ffmpeg_stats_on_carriage_return() {
        let input = b"Duration: 00:00:20.00\r\nframe= 1 time=00:00:01.00\rframe= 2 time=00:00:02.00\rdone\n\nlast";
        let lines: Vec<String> = CrLfLines::new(&input[..]).collect();
        assert_eq!(
            lines,
            vec![
                "Duration: 00:00:20.00",
                "frame= 1 time=00:00:01.00",
                "frame= 2 time=00:00:02.00",
                "done",
                "last"
            ]
        );
    }

    #[test]
    fn eta_waits_for_movement_then_tracks_rate() {
        let t0 = Instant::now();
        let mut origin = None;
        assert_eq!(estimate_eta(&mut origin, 10.0, t0), None);
        assert_eq!(estimate_eta(&mut origin, 10.2, t0 + Duration::from_secs(1)), None);
        let eta = estimate_eta(&mut origin, 20.0, t0 + Duration::from_secs(10)).unwrap();
        assert!((eta - 80.0).abs() < 1e-6, "{eta}");
        // Progress going backwards (second yt-dlp stream) restarts the measurement.
        assert_eq!(estimate_eta(&mut origin, 5.0, t0 + Duration::from_secs(11)), None);
        assert_eq!(estimate_eta(&mut origin, 100.0, t0 + Duration::from_secs(12)), None);
    }

    #[test]
    fn ensure_output_parent_creates_nested_directory() {
        let base = std::env::temp_dir().join(format!("fc-out-test-{}", Uuid::new_v4()));
        let out = base.join("nested").join("file.png");
        ensure_output_parent(&out).expect("mkdir should succeed");
        assert!(base.join("nested").is_dir());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn output_path_uses_custom_directory() {
        let input = "C:/videos/clip.mp4";
        let custom = Path::new("D:/exports");
        let out = output_path(input, Some(custom), "mkv", None);
        assert_eq!(out, Path::new("D:/exports/clip.mkv"));
    }

    #[test]
    fn output_path_from_m3u8_url() {
        let url = "https://cdn.example.com/vod/index-dvr.m3u8";
        let custom = Path::new("D:/exports");
        let out = output_path(url, Some(custom), "mp4", None);
        assert_eq!(out, Path::new("D:/exports/index-dvr.mp4"));
    }

    #[test]
    fn output_path_preserves_brackets_and_underscores() {
        let input = r"C:/music/[FREE] beat - _name_.mp3";
        let out = output_path(input, None, "ogg", None);
        assert_eq!(
            out,
            Path::new(r"C:/music/[FREE] beat - _name_.ogg")
        );
    }

    #[test]
    fn output_path_sanitizes_invalid_windows_chars() {
        let input = "C:/music/[free] beat \u{201C}bridge\u{201D}.mp3";
        let out = output_path(input, None, "ogg", None);
        assert_eq!(
            out,
            Path::new("C:/music/[free] beat _bridge_.ogg")
        );
    }

    #[test]
    fn remote_url_defaults_to_downloads() {
        let url = "https://www.twitch.tv/x/clip/Abc";
        let out = output_path(url, None, "mp4", Some("my clip"));
        assert_eq!(out.parent(), Some(default_downloads_dir().as_path()));
        assert_eq!(out.file_name().and_then(|s| s.to_str()), Some("my clip.mp4"));
    }

    #[test]
    fn local_file_defaults_to_source_parent() {
        let input = "C:/videos/folder/clip.mp4";
        let out = output_path(input, None, "mkv", None);
        assert_eq!(out, Path::new("C:/videos/folder/clip.mkv"));
    }
}
