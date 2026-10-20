//! Command-line front end (`fconv`). Uses the same probing, validation and conversion
//! queue as the window; events are printed instead of sent to the UI.

use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use clap::{Args, Parser, Subcommand, ValueEnum};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};

use crate::commands::{build_job_info, validate_jobs, FileJobInfo};
use crate::format_registry::{
    all_supported_extensions, is_supported_input, normalize_ext, outputs_for_input,
    ConversionTier, MediaCategory,
};
use crate::hls::{effective_input_ext, is_hls_source, is_m3u8_url};
use crate::hls_playlist::parse_time_value;
use crate::queue::{
    cancel, run_queue, CancelledEvent, CompleteEvent, ConversionJob, ErrorEvent, JobSink,
    ProgressEvent, QueueState, SkippedEvent,
};
use crate::youtube::is_ytdlp_url;
use crate::{ffmpeg_tools, ytdlp_tools};

const EXIT_OK: i32 = 0;
const EXIT_FAILED: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_TOOLS_MISSING: i32 = 3;
const EXIT_CANCELLED: i32 = 130;

#[derive(Parser)]
#[command(
    name = "fconv",
    bin_name = "fconv",
    version,
    about = "File Converter CLI: convert video, audio and images, download media links, and cut HLS streams.",
    after_help = "Examples:\n  \
        fconv convert clip.mkv -f mp4\n  \
        fconv convert C:\\Music -r -f mp3 -o D:\\Converted\n  \
        fconv convert \"https://www.youtube.com/watch?v=...\" -q 1080\n  \
        fconv convert \"https://.../index-dvr.m3u8\" --start 5:00:00 --end 6:00:00 -n stream-hour-6\n  \
        fconv info photo.png\n  \
        fconv formats --category audio"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert local files, download media links, or cut HLS (.m3u8) streams.
    Convert(ConvertArgs),
    /// Show detected type, conversion mode and valid output formats for inputs.
    Info(InfoArgs),
    /// List supported input extensions and output formats.
    Formats(FormatsArgs),
    /// Verify FFmpeg, ffprobe and yt-dlp are available.
    Check(JsonFlag),
    /// Update the bundled yt-dlp to the latest release.
    UpdateYtdlp,
}

#[derive(Args)]
struct JsonFlag {
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct InputArgs {
    /// Files, folders, media links, or .m3u8 URLs.
    inputs: Vec<String>,
    /// Read additional inputs from a file, one per line ("-" for stdin).
    #[arg(long, value_name = "FILE")]
    from_file: Option<String>,
    /// Include files in subfolders when an input is a folder.
    #[arg(short, long)]
    recursive: bool,
}

#[derive(Args)]
struct ConvertArgs {
    #[command(flatten)]
    input: InputArgs,
    /// Output format (e.g. mp4, mkv, mp3, png). Default: mp4 for video, mp3 for audio,
    /// png for images.
    #[arg(short, long, value_name = "EXT")]
    format: Option<String>,
    /// Output folder (created if missing). Default: next to local files; Downloads for links.
    #[arg(short, long, value_name = "DIR")]
    output_dir: Option<String>,
    /// Output file name without extension (single input only).
    #[arg(short, long, value_name = "NAME")]
    name: Option<String>,
    /// HLS start time: seconds, MM:SS or HH:MM:SS (absolute position in the stream).
    #[arg(long, value_name = "TIME")]
    start: Option<String>,
    /// HLS end time: seconds, MM:SS or HH:MM:SS (absolute position in the stream).
    #[arg(long, value_name = "TIME")]
    end: Option<String>,
    /// Link quality: best, audio, or a max height such as 1080 or 720.
    #[arg(short, long, value_name = "QUALITY")]
    quality: Option<String>,
    /// Replace existing output files.
    #[arg(short = 'y', long)]
    overwrite: bool,
    /// Print one JSON event per line on stdout instead of a progress bar.
    #[arg(long, conflicts_with = "quiet")]
    json: bool,
    /// Only print output paths and errors.
    #[arg(long)]
    quiet: bool,
}

#[derive(Args)]
struct InfoArgs {
    #[command(flatten)]
    input: InputArgs,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum CategoryArg {
    Video,
    Audio,
    Image,
}

impl From<CategoryArg> for MediaCategory {
    fn from(c: CategoryArg) -> Self {
        match c {
            CategoryArg::Video => MediaCategory::Video,
            CategoryArg::Audio => MediaCategory::Audio,
            CategoryArg::Image => MediaCategory::Image,
        }
    }
}

#[derive(Args)]
struct FormatsArgs {
    /// Only show outputs for this input category.
    #[arg(long, value_enum)]
    category: Option<CategoryArg>,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Parse `std::env::args()` and run. Returns the process exit code.
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { EXIT_USAGE } else { EXIT_OK };
        }
    };

    ffmpeg_tools::init_standalone();
    ytdlp_tools::init_standalone();

    match cli.command {
        Command::Convert(args) => run_convert(args),
        Command::Info(args) => run_info(args),
        Command::Formats(args) => run_formats(args),
        Command::Check(args) => run_check(args.json),
        Command::UpdateYtdlp => {
            eprintln!("Updating yt-dlp at {} ...", ytdlp_tools::ytdlp_path().display());
            ytdlp_tools::self_update_blocking();
            EXIT_OK
        }
    }
}

fn fail_usage(message: &str) -> i32 {
    eprintln!("error: {message}");
    EXIT_USAGE
}

// ---------------------------------------------------------------- inputs

fn is_remote(input: &str) -> bool {
    is_m3u8_url(input) || is_ytdlp_url(input)
}

/// Expand folders, read `--from-file`, and make local paths absolute (the path validator
/// rejects `..` components).
fn collect_inputs(args: &InputArgs) -> Result<Vec<String>, String> {
    let mut raw: Vec<String> = args.inputs.clone();
    if let Some(list) = &args.from_file {
        let lines: Vec<String> = if list == "-" {
            std::io::stdin().lock().lines().map_while(Result::ok).collect()
        } else {
            std::fs::read_to_string(list)
                .map_err(|e| format!("Cannot read {list}: {e}"))?
                .lines()
                .map(str::to_string)
                .collect()
        };
        raw.extend(
            lines
                .into_iter()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#')),
        );
    }
    if raw.is_empty() {
        return Err("No inputs given.".to_string());
    }

    let mut out = Vec::new();
    for item in raw {
        let item = item.trim().trim_matches('"').to_string();
        if is_remote(&item) {
            out.push(item);
            continue;
        }
        let abs = std::path::absolute(&item).map_err(|e| format!("{item}: {e}"))?;
        if abs.is_dir() {
            let before = out.len();
            collect_dir(&abs, args.recursive, &mut out)?;
            if out.len() == before {
                eprintln!("warning: no supported files in {}", abs.display());
            }
        } else {
            out.push(abs.to_string_lossy().into_owned());
        }
    }
    if out.is_empty() {
        return Err("No supported inputs found.".to_string());
    }
    Ok(out)
}

fn collect_dir(dir: &Path, recursive: bool, out: &mut Vec<String>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read folder {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if recursive {
                collect_dir(&path, true, out)?;
            }
        } else if is_supported_input(&effective_input_ext(&path.to_string_lossy())) {
            out.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

fn probe_inputs(inputs: &[String], format: Option<&str>) -> Vec<FileJobInfo> {
    inputs
        .par_iter()
        .map(|p| build_job_info(p.clone(), format.map(str::to_string)))
        .collect()
}

// ---------------------------------------------------------------- convert

fn run_convert(args: ConvertArgs) -> i32 {
    if let Err(e) = ffmpeg_tools::check_available() {
        eprintln!("error: {e}");
        return EXIT_TOOLS_MISSING;
    }
    crate::converter::warm_encoder_cache();

    let inputs = match collect_inputs(&args.input) {
        Ok(v) => v,
        Err(e) => return fail_usage(&e),
    };
    if args.name.is_some() && inputs.len() > 1 {
        return fail_usage("--name can only be used with a single input.");
    }

    let parse_opt_time = |v: &Option<String>, flag: &str| -> Result<Option<f64>, String> {
        match v {
            Some(s) => parse_time_value(s).map_err(|e| format!("--{flag}: {e}")),
            None => Ok(None),
        }
    };
    let start = match parse_opt_time(&args.start, "start") {
        Ok(v) => v,
        Err(e) => return fail_usage(&e),
    };
    let end = match parse_opt_time(&args.end, "end") {
        Ok(v) => v,
        Err(e) => return fail_usage(&e),
    };
    let has_range = start.is_some() || end.is_some();
    if has_range
        && !inputs
            .iter()
            .any(|i| is_hls_source(i, &effective_input_ext(i)))
    {
        return fail_usage("--start/--end apply to HLS (.m3u8) inputs only.");
    }
    let quality = args.quality.as_deref().map(|q| q.trim().to_lowercase());
    if quality.is_some() && !inputs.iter().any(|i| is_ytdlp_url(i)) {
        return fail_usage("--quality applies to media links only.");
    }

    let output_dir = match &args.output_dir {
        Some(dir) => match std::fs::create_dir_all(dir).and_then(|_| std::path::absolute(dir)) {
            Ok(abs) => Some(abs.to_string_lossy().into_owned()),
            Err(e) => return fail_usage(&format!("--output-dir {dir}: {e}")),
        },
        None => None,
    };

    let mode = if args.json {
        OutputMode::Json
    } else if args.quiet {
        OutputMode::Quiet
    } else {
        OutputMode::Human
    };

    if mode == OutputMode::Human {
        eprintln!("Checking {} input(s)...", inputs.len());
    }
    let infos = probe_inputs(&inputs, args.format.as_deref());

    let mut failures = 0usize;
    let mut jobs = Vec::new();
    let mut names = HashMap::new();
    for info in infos {
        if let Some(problem) = job_problem(&info, args.format.as_deref(), quality.as_deref()) {
            failures += 1;
            report_input_error(mode, &info.path, &problem);
            continue;
        }
        let is_hls = is_hls_source(&info.path, &effective_input_ext(&info.path));
        let is_link = is_ytdlp_url(&info.path);
        names.insert(info.id.clone(), (info.path.clone(), info.name.clone()));
        jobs.push(ConversionJob {
            id: info.id,
            path: info.path,
            output_format: info.output_format,
            hls_start_secs: if is_hls && has_range { start } else { info.hls_start_secs },
            hls_end_secs: if is_hls && has_range { end } else { info.hls_end_secs },
            youtube_quality: if is_link {
                Some(quality.clone().unwrap_or_else(|| "best".to_string()))
            } else {
                None
            },
            output_name: args.name.clone(),
        });
    }

    if jobs.is_empty() {
        return EXIT_FAILED;
    }

    let (jobs, output_dir) = match validate_jobs(jobs, output_dir.as_deref(), args.overwrite) {
        Ok(v) => v,
        Err(e) if e.contains("already exist") => {
            return fail_usage(&format!("{e} Use --overwrite (-y) to replace them."));
        }
        Err(e) => return fail_usage(&e),
    };

    let order: Vec<String> = jobs.iter().map(|j| j.id.clone()).collect();
    let sink = Arc::new(CliSink::new(mode, names, order));
    let state = Arc::new(QueueState::new());
    install_ctrlc(state.clone(), mode);

    let Some(handle) = run_queue(
        sink.clone(),
        jobs,
        output_dir.map(|p| p.to_string_lossy().into_owned()),
        state.clone(),
    ) else {
        return EXIT_FAILED;
    };
    let _ = handle.join();

    let summary = sink.summary();
    failures += summary.failed;
    if mode == OutputMode::Human && summary.total > 1 {
        eprintln!(
            "\n{} done, {} failed, {} cancelled in {}",
            summary.done,
            failures,
            summary.cancelled,
            format_duration(summary.wall_secs)
        );
    }
    if state.cancel.load(Ordering::SeqCst) {
        EXIT_CANCELLED
    } else if failures > 0 {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

/// Reasons a probed input cannot be queued with the requested options.
fn job_problem(info: &FileJobInfo, format: Option<&str>, quality: Option<&str>) -> Option<String> {
    if let Some(e) = &info.error {
        return Some(e.clone());
    }
    if let Some(fmt) = format {
        if normalize_ext(fmt) != normalize_ext(&info.output_format) {
            let valid = valid_outputs(info.category).join(", ");
            return Some(format!(
                "Cannot convert {} input to .{}. Valid formats: {valid}",
                category_label(info.category),
                normalize_ext(fmt)
            ));
        }
    }
    if let (Some(q), Some(available)) = (quality, &info.youtube_qualities) {
        if q != "best" && !available.iter().any(|a| a.value == q) {
            let list: Vec<&str> = available.iter().map(|a| a.value.as_str()).collect();
            return Some(format!(
                "Quality \"{q}\" is not available. Choose from: best, {}",
                list.join(", ")
            ));
        }
    }
    None
}

fn report_input_error(mode: OutputMode, input: &str, message: &str) {
    match mode {
        OutputMode::Json => print_json(json!({
            "event": "error",
            "input": input,
            "message": message,
        })),
        _ => eprintln!("FAILED  {input}\n        {message}"),
    }
}

fn install_ctrlc(state: Arc<QueueState>, mode: OutputMode) {
    let presses = AtomicUsize::new(0);
    let _ = ctrlc::set_handler(move || {
        if presses.fetch_add(1, Ordering::SeqCst) > 0 {
            std::process::exit(EXIT_CANCELLED);
        }
        if mode != OutputMode::Json {
            eprintln!("\nCancelling... (press Ctrl+C again to force quit)");
        }
        cancel(&state);
    });
}

// ---------------------------------------------------------------- event output

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Human,
    Json,
    Quiet,
}

#[derive(Default, Clone, Copy)]
struct Summary {
    total: usize,
    done: usize,
    failed: usize,
    cancelled: usize,
    wall_secs: f64,
}

struct SinkState {
    /// Length of the progress line currently drawn on the terminal (0 = none).
    line_len: usize,
    /// Last printed 10% bucket per job, for non-terminal progress output.
    last_bucket: HashMap<String, i64>,
    summary: Summary,
}

struct CliSink {
    mode: OutputMode,
    tty: bool,
    /// job id -> (input, display name)
    names: HashMap<String, (String, String)>,
    order: Vec<String>,
    started: std::time::Instant,
    state: Mutex<SinkState>,
}

impl CliSink {
    fn new(mode: OutputMode, names: HashMap<String, (String, String)>, order: Vec<String>) -> Self {
        let total = order.len();
        Self {
            mode,
            tty: std::io::stderr().is_terminal(),
            names,
            order,
            started: std::time::Instant::now(),
            state: Mutex::new(SinkState {
                line_len: 0,
                last_bucket: HashMap::new(),
                summary: Summary {
                    total,
                    ..Summary::default()
                },
            }),
        }
    }

    fn summary(&self) -> Summary {
        let st = self.state.lock().unwrap();
        Summary {
            wall_secs: self.started.elapsed().as_secs_f64(),
            ..st.summary
        }
    }

    fn label(&self, id: &str) -> String {
        let pos = self.order.iter().position(|o| o == id).map_or(0, |p| p + 1);
        let name = self.names.get(id).map_or("", |(_, n)| n.as_str());
        format!("[{pos}/{}] {}", self.order.len(), truncate(name, 48))
    }

    fn input(&self, id: &str) -> &str {
        self.names.get(id).map_or("", |(i, _)| i.as_str())
    }

    fn json_event(&self, kind: &str, id: &str, event: &impl Serialize) {
        let mut value = serde_json::to_value(event).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut value {
            map.insert("event".into(), json!(kind));
            map.insert("input".into(), json!(self.input(id)));
        }
        print_json(value);
    }

    /// Print a full line, clearing any in-place progress line first.
    fn println(&self, st: &mut SinkState, text: &str) {
        let mut err = std::io::stderr().lock();
        if st.line_len > 0 {
            let _ = write!(err, "\r{}\r", " ".repeat(st.line_len));
            st.line_len = 0;
        }
        let _ = writeln!(err, "{text}");
    }
}

impl JobSink for CliSink {
    fn progress(&self, e: ProgressEvent) {
        match self.mode {
            OutputMode::Json => self.json_event("progress", &e.id, &e),
            OutputMode::Quiet => {}
            OutputMode::Human => {
                let mut timing = format!("elapsed {}", format_duration(e.elapsed_secs));
                if let Some(eta) = e.eta_secs {
                    timing.push_str(&format!("  ETA {}", format_duration(eta)));
                }
                let line = format!(
                    "{}  {:>5.1}%  {}  {}",
                    self.label(&e.id),
                    e.progress,
                    timing,
                    e.status
                );
                let mut st = self.state.lock().unwrap();
                if self.tty {
                    let line = truncate(&line, 119);
                    let pad = st.line_len.saturating_sub(line.chars().count());
                    let mut err = std::io::stderr().lock();
                    let _ = write!(err, "\r{line}{}", " ".repeat(pad));
                    let _ = err.flush();
                    st.line_len = line.chars().count();
                } else {
                    let bucket = (e.progress / 10.0).floor() as i64;
                    if st.last_bucket.get(&e.id) != Some(&bucket) {
                        st.last_bucket.insert(e.id.clone(), bucket);
                        self.println(&mut st, &line);
                    }
                }
            }
        }
    }

    fn complete(&self, e: CompleteEvent) {
        let mut st = self.state.lock().unwrap();
        st.summary.done += 1;
        match self.mode {
            OutputMode::Json => self.json_event("complete", &e.id, &e),
            OutputMode::Quiet => println!("{}", e.output_path),
            OutputMode::Human => {
                let text = format!(
                    "DONE    {}  ({})\n        -> {}",
                    self.label(&e.id),
                    format_duration(e.elapsed_secs),
                    e.output_path
                );
                self.println(&mut st, &text);
            }
        }
    }

    fn error(&self, e: ErrorEvent) {
        let mut st = self.state.lock().unwrap();
        st.summary.failed += 1;
        match self.mode {
            OutputMode::Json => self.json_event("error", &e.id, &e),
            _ => {
                let text = format!("FAILED  {}\n        {}", self.label(&e.id), e.message);
                self.println(&mut st, &text);
            }
        }
    }

    fn cancelled(&self, e: CancelledEvent) {
        let mut st = self.state.lock().unwrap();
        st.summary.cancelled += 1;
        match self.mode {
            OutputMode::Json => self.json_event("cancelled", &e.id, &e),
            _ => {
                let text = format!("CANCEL  {}", self.label(&e.id));
                self.println(&mut st, &text);
            }
        }
    }

    fn skipped(&self, e: SkippedEvent) {
        let mut st = self.state.lock().unwrap();
        st.summary.cancelled += 1;
        match self.mode {
            OutputMode::Json => self.json_event("skipped", &e.id, &e),
            OutputMode::Quiet => {}
            OutputMode::Human => {
                let text = format!("SKIP    {}  {}", self.label(&e.id), e.message);
                self.println(&mut st, &text);
            }
        }
    }

    fn queue_complete(&self) {
        if self.mode == OutputMode::Json {
            let st = self.state.lock().unwrap();
            print_json(json!({
                "event": "queue_complete",
                "done": st.summary.done,
                "failed": st.summary.failed,
                "cancelled": st.summary.cancelled,
                "elapsed_secs": self.started.elapsed().as_secs_f64(),
            }));
        }
    }
}

// ---------------------------------------------------------------- info / formats / check

fn run_info(args: InfoArgs) -> i32 {
    let inputs = match collect_inputs(&args.input) {
        Ok(v) => v,
        Err(e) => return fail_usage(&e),
    };
    let infos = probe_inputs(&inputs, None);
    let any_error = infos.iter().any(|i| i.error.is_some());

    if args.json {
        let list: Vec<Value> = infos
            .iter()
            .map(|i| {
                json!({
                    "input": i.path,
                    "name": i.name,
                    "category": i.category,
                    "mode": tier_label(i.tier),
                    "default_format": i.output_format,
                    "output_formats": outputs_for_input(i.category),
                    "platform": i.platform,
                    "qualities": i.youtube_qualities,
                    "hls_start_secs": i.hls_start_secs,
                    "hls_end_secs": i.hls_end_secs,
                    "error": i.error,
                })
            })
            .collect();
        print_json(Value::Array(list));
    } else {
        for (n, i) in infos.iter().enumerate() {
            if n > 0 {
                println!();
            }
            println!("{}", i.name);
            println!("  input     {}", i.path);
            if let Some(err) = &i.error {
                println!("  error     {err}");
                continue;
            }
            println!("  type      {}", category_label(i.category));
            if let Some(p) = &i.platform {
                println!("  platform  {p}");
            }
            println!("  default   .{} ({})", i.output_format, tier_label(i.tier));
            for group in outputs_for_input(i.category) {
                println!("  {:<9} {}", short_group(&group.label), group.formats.join(", "));
            }
            if let Some(q) = &i.youtube_qualities {
                let list: Vec<String> = q
                    .iter()
                    .map(|q| format!("{} ({})", q.value, q.label))
                    .collect();
                println!("  quality   best, {}", list.join(", "));
            }
        }
    }
    if any_error {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

fn run_formats(args: FormatsArgs) -> i32 {
    let categories: Vec<MediaCategory> = match args.category {
        Some(c) => vec![c.into()],
        None => vec![MediaCategory::Video, MediaCategory::Audio, MediaCategory::Image],
    };
    if args.json {
        let outputs: serde_json::Map<String, Value> = categories
            .iter()
            .map(|c| (category_label(*c).to_lowercase(), json!(outputs_for_input(*c))))
            .collect();
        print_json(json!({
            "inputs": all_supported_extensions(),
            "outputs": outputs,
        }));
        return EXIT_OK;
    }
    if args.category.is_none() {
        println!("Input extensions: {}", all_supported_extensions().join(", "));
        println!("Also: media links (YouTube, Twitch, TikTok, X, Instagram, ...) and .m3u8 URLs\n");
    }
    for c in categories {
        println!("{} inputs can be converted to:", category_label(c));
        for group in outputs_for_input(c) {
            println!("  {:<16} {}", group.label, group.formats.join(", "));
        }
    }
    EXIT_OK
}

fn run_check(as_json: bool) -> i32 {
    let ffmpeg = ffmpeg_tools::check_available();
    let ytdlp = ytdlp_tools::check_available();
    let js = ytdlp_tools::js_runtime();
    if as_json {
        print_json(json!({
            "ffmpeg": { "path": ffmpeg_tools::ffmpeg_path(), "ok": ffmpeg.is_ok(), "error": ffmpeg.as_ref().err() },
            "ffprobe": { "path": ffmpeg_tools::ffprobe_path(), "ok": ffmpeg.is_ok() },
            "yt_dlp": { "path": ytdlp_tools::ytdlp_path(), "ok": ytdlp.is_ok(), "error": ytdlp.as_ref().err() },
            "js_runtime": js.as_ref().map(|(name, path)| json!({ "name": name, "path": path })),
        }));
    } else {
        let status = |r: &Result<(), String>| match r {
            Ok(()) => "OK".to_string(),
            Err(e) => format!("MISSING ({e})"),
        };
        println!("ffmpeg      {}  {}", status(&ffmpeg), ffmpeg_tools::ffmpeg_path().display());
        println!("ffprobe     {}  {}", status(&ffmpeg), ffmpeg_tools::ffprobe_path().display());
        println!("yt-dlp      {}  {}", status(&ytdlp), ytdlp_tools::ytdlp_path().display());
        match &js {
            Some((name, path)) => println!("js runtime  OK  {name} ({})", path.display()),
            None => println!("js runtime  not found (only needed for YouTube; install Node.js or Deno)"),
        }
    }
    if ffmpeg.is_ok() && ytdlp.is_ok() {
        EXIT_OK
    } else {
        EXIT_TOOLS_MISSING
    }
}

// ---------------------------------------------------------------- helpers

fn print_json(value: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn valid_outputs(category: MediaCategory) -> Vec<String> {
    outputs_for_input(category)
        .into_iter()
        .flat_map(|g| g.formats)
        .collect()
}

fn category_label(category: MediaCategory) -> &'static str {
    match category {
        MediaCategory::Video => "Video",
        MediaCategory::Audio => "Audio",
        MediaCategory::Image => "Image",
        MediaCategory::Unknown => "Unknown",
    }
}

fn tier_label(tier: ConversionTier) -> &'static str {
    match tier {
        ConversionTier::Copy => "stream copy, no quality loss",
        ConversionTier::EncodeHq => "high-quality encode",
    }
}

fn short_group(label: &str) -> String {
    if label.starts_with("Audio (") {
        "extract".to_string()
    } else {
        label.to_lowercase()
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// `45s`, `3m 07s`, `1h 02m 09s` — same style as the window.
pub fn format_duration(total_secs: f64) -> String {
    let secs = total_secs.max(0.0).round() as u64;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations_like_the_window() {
        assert_eq!(format_duration(0.4), "0s");
        assert_eq!(format_duration(45.0), "45s");
        assert_eq!(format_duration(187.0), "3m 07s");
        assert_eq!(format_duration(3729.0), "1h 02m 09s");
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_convert_flags() {
        let cli = Cli::try_parse_from([
            "fconv", "convert", "a.mkv", "-f", "mp4", "-o", "out", "-y", "--start", "1:00",
        ])
        .unwrap();
        let Command::Convert(args) = cli.command else {
            panic!("expected convert");
        };
        assert_eq!(args.input.inputs, vec!["a.mkv"]);
        assert_eq!(args.format.as_deref(), Some("mp4"));
        assert_eq!(args.output_dir.as_deref(), Some("out"));
        assert!(args.overwrite);
        assert_eq!(args.start.as_deref(), Some("1:00"));
    }

    #[test]
    fn json_and_quiet_conflict() {
        assert!(Cli::try_parse_from(["fconv", "convert", "a.mkv", "--json", "--quiet"]).is_err());
    }

    #[test]
    fn rejects_wrong_format_for_category() {
        let info = FileJobInfo {
            id: "1".into(),
            path: "C:/a.mp3".into(),
            name: "a.mp3".into(),
            category: MediaCategory::Audio,
            tier: ConversionTier::EncodeHq,
            output_format: "mp3".into(),
            hls_start_secs: None,
            hls_end_secs: None,
            youtube_quality: None,
            youtube_qualities: None,
            platform: None,
            error: None,
        };
        assert!(job_problem(&info, Some("mp4"), None).is_some());
        assert!(job_problem(&info, Some(".MP3"), None).is_none());
        assert!(job_problem(&info, None, None).is_none());
    }

    #[test]
    fn collects_supported_files_from_folder() {
        let dir = std::env::temp_dir().join(format!("fc-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["a.mp4", "b.txt", "sub/c.mp3"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let args = |recursive| InputArgs {
            inputs: vec![dir.to_string_lossy().into_owned()],
            from_file: None,
            recursive,
        };
        let flat = collect_inputs(&args(false)).unwrap();
        assert_eq!(flat.len(), 1);
        assert!(flat[0].ends_with("a.mp4"));
        assert_eq!(collect_inputs(&args(true)).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
