use std::sync::OnceLock;

use crate::ffmpeg_tools;
use crate::process_utils::hidden_command;
use crate::format_registry::{
    is_lossless_extension, normalize_ext, output_category, ConversionTier, MediaCategory,
};
use crate::hls::{is_hls_source, is_m3u8_url, media_input_path, protocol_whitelist_for};
use crate::probe::ProbeResult;

static ENCODER_CAPS: OnceLock<EncoderCaps> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct HwEncoders {
    pub nvenc: bool,
    pub amf: bool,
    pub qsv: bool,
}

#[derive(Debug, Clone)]
pub struct ImageEncoders {
    pub svt_av1: bool,
    pub libaom_av1: bool,
}

#[derive(Debug, Clone)]
pub struct CpuEncoders {
    pub libx264: bool,
    pub libx265: bool,
    pub libsvtav1: bool,
    pub libvpx_vp9: bool,
}

#[derive(Debug, Clone)]
pub struct EncoderCaps {
    pub hw: HwEncoders,
    pub image: ImageEncoders,
    pub cpu: CpuEncoders,
}

static DEFAULT_IMAGE: ImageEncoders = ImageEncoders {
    svt_av1: false,
    libaom_av1: false,
};

fn ffmpeg_encoder_list() -> String {
    hidden_command(ffmpeg_tools::ffmpeg_path())
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn detect_encoder_caps() -> EncoderCaps {
    let text = ffmpeg_encoder_list();
    EncoderCaps {
        hw: HwEncoders {
            nvenc: text.contains("h264_nvenc"),
            amf: text.contains("h264_amf"),
            qsv: text.contains("h264_qsv"),
        },
        image: ImageEncoders {
            svt_av1: text.contains("libsvtav1"),
            libaom_av1: text.contains("libaom-av1"),
        },
        cpu: CpuEncoders {
            libx264: text.contains("libx264"),
            libx265: text.contains("libx265"),
            libsvtav1: text.contains("libsvtav1"),
            libvpx_vp9: text.contains("libvpx-vp9"),
        },
    }
}

/// Detect encoder capabilities now (CLI: no UI thread to protect).
pub fn warm_encoder_cache() {
    let _ = ENCODER_CAPS.get_or_init(detect_encoder_caps);
}

/// Warm encoder capabilities in the background. Never blocks IPC or the UI thread.
pub fn init_encoder_cache_background() {
    std::thread::spawn(|| {
        let _ = ENCODER_CAPS.get_or_init(detect_encoder_caps);
    });
}

/// Blocks on first use; only call from conversion threads.
pub fn get_hw_encoders_blocking() -> &'static HwEncoders {
    &ENCODER_CAPS.get_or_init(detect_encoder_caps).hw
}

/// Blocks on first use (one `ffmpeg -encoders` spawn); only call from conversion threads.
pub fn get_cpu_encoders() -> &'static CpuEncoders {
    &ENCODER_CAPS.get_or_init(detect_encoder_caps).cpu
}

pub fn get_image_encoders() -> &'static ImageEncoders {
    ENCODER_CAPS.get().map(|c| &c.image).unwrap_or(&DEFAULT_IMAGE)
}

pub fn avif_encode_available(image_encoders: &ImageEncoders) -> bool {
    image_encoders.svt_av1 || image_encoders.libaom_av1
}

/// Allow AVIF while encoder cache is loading; reject only once loaded and unavailable.
pub fn avif_output_allowed() -> bool {
    match ENCODER_CAPS.get() {
        Some(caps) => avif_encode_available(&caps.image),
        None => true,
    }
}

pub fn build_ffmpeg_args(
    input: &str,
    output: &str,
    probe: &ProbeResult,
    input_ext: &str,
    output_ext: &str,
    tier: ConversionTier,
    hw: &HwEncoders,
    cpu: &CpuEncoders,
    cpu_only: bool,
) -> Vec<String> {
    let output_ext = normalize_ext(output_ext);
    let out_cat = output_category(&output_ext);
    let hls = is_hls_source(input, input_ext);
    let mut args = vec![
        ffmpeg_tools::ffmpeg_path().to_string_lossy().into_owned(),
        "-y".to_string(),
    ];

    if hls {
        args.push("-protocol_whitelist".to_string());
        args.push(protocol_whitelist_for(input).to_string());
        // HTTP reconnect options only apply when the primary input is http(s).
        // Ranged jobs use a local temp .m3u8; passing -reconnect there makes
        // ffmpeg exit with "Option not found".
        if is_m3u8_url(input) {
            args.push("-reconnect".to_string());
            args.push("1".to_string());
            args.push("-reconnect_streamed".to_string());
            args.push("1".to_string());
            args.push("-reconnect_on_network_error".to_string());
            args.push("1".to_string());
            args.push("-reconnect_on_http_error".to_string());
            args.push("4xx,5xx".to_string());
        }
        args.push("-fflags".to_string());
        args.push("+genpts".to_string());
    }

    // Input options: must precede -i or ffmpeg rejects them as output options.
    if !cpu_only
        && !hls
        && tier == ConversionTier::EncodeHq
        && probe.category == MediaCategory::Video
        && out_cat != MediaCategory::Image
        && output_ext != "gif"
        && probe
            .video_codec
            .as_deref()
            .is_some_and(supports_d3d11va_decode)
    {
        args.push("-hwaccel".to_string());
        args.push("d3d11va".to_string());
    }

    args.push("-i".to_string());
    args.push(media_input_path(input));

    match out_cat {
        MediaCategory::Video if output_ext == "gif" => {
            build_video_to_gif(&mut args);
        }
        MediaCategory::Audio if probe.category == MediaCategory::Video => {
            args.push("-vn".to_string());
            if tier == ConversionTier::Copy {
                args.push("-c:a".to_string());
                args.push("copy".to_string());
            } else {
                append_audio_encode(&mut args, &output_ext);
            }
        }
        MediaCategory::Audio => {
            if tier == ConversionTier::Copy {
                args.push("-c".to_string());
                args.push("copy".to_string());
            } else {
                append_audio_encode(&mut args, &output_ext);
            }
        }
        MediaCategory::Image => {
            build_image_args(
                &mut args,
                input_ext,
                &output_ext,
                probe.is_lossless_source,
                get_image_encoders(),
            );
        }
        MediaCategory::Video => {
            if tier == ConversionTier::Copy {
                args.push("-c".to_string());
                args.push("copy".to_string());
                if hls {
                    args.push("-avoid_negative_ts".to_string());
                    args.push("make_zero".to_string());
                }
                if hls && (output_ext == "mp4" || output_ext == "mov") {
                    args.push("-bsf:a".to_string());
                    args.push("aac_adtstoasc".to_string());
                }
            } else {
                if cpu_only {
                    append_video_encode_cpu(&mut args, &output_ext, cpu);
                } else {
                    append_video_encode(&mut args, &output_ext, hw, cpu);
                }
                if probe.has_audio {
                    let audio_ext = default_audio_for_video(&output_ext);
                    append_audio_encode(&mut args, &audio_ext);
                } else {
                    args.push("-an".to_string());
                }
            }
            if output_ext == "mp4" || output_ext == "mov" {
                args.push("-movflags".to_string());
                args.push("+faststart".to_string());
            }
        }
        MediaCategory::Unknown => {}
    }

    args.push(output.to_string());
    args
}

/// `-hwaccel auto` can pick Vulkan compute decoders (e.g. ProRes), which hang NVIDIA
/// drivers (nvlddmkm TDR). Only use D3D11VA for codecs with fixed-function decoders.
fn supports_d3d11va_decode(codec: &str) -> bool {
    matches!(
        codec.to_lowercase().as_str(),
        "h264" | "hevc" | "vp9" | "av1" | "mpeg2video"
    )
}

pub fn looks_like_hw_encoder_failure(stderr: &[String]) -> bool {
    let combined = stderr.join("\n").to_lowercase();
    combined.contains("nvenc")
        || combined.contains("_amf")
        || combined.contains("_qsv")
        || combined.contains("cuda")
        || combined.contains("no capable devices")
        || combined.contains("cannot load")
        || combined.contains("error while opening encoder")
}

fn default_audio_for_video(video_ext: &str) -> String {
    if normalize_ext(video_ext) == "webm" {
        "ogg".to_string()
    } else {
        "aac".to_string()
    }
}

fn append_video_encode_cpu(args: &mut Vec<String>, output_ext: &str, cpu: &CpuEncoders) {
    args.push("-threads".to_string());
    args.push("0".to_string());
    append_software_video_codec(args, output_ext, cpu);
}

/// WebM only allows VP8/VP9/AV1 video, so H.264/HEVC encoders cannot be used.
fn append_webm_video_codec(args: &mut Vec<String>, cpu: &CpuEncoders) {
    if cpu.libsvtav1 || !cpu.libvpx_vp9 {
        args.extend(
            ["-c:v", "libsvtav1", "-crf", "26", "-preset", "7", "-pix_fmt", "yuv420p"]
                .map(String::from),
        );
    } else {
        args.extend(
            [
                "-c:v", "libvpx-vp9", "-crf", "24", "-b:v", "0", "-row-mt", "1",
                "-deadline", "good", "-cpu-used", "4",
            ]
            .map(String::from),
        );
    }
}

fn append_software_video_codec(args: &mut Vec<String>, output_ext: &str, cpu: &CpuEncoders) {
    let output_ext = normalize_ext(output_ext);
    if output_ext == "webm" {
        append_webm_video_codec(args, cpu);
    } else if output_ext == "mkv" && cpu.libx265 {
        args.push("-c:v".to_string());
        args.push("libx265".to_string());
        args.push("-crf".to_string());
        args.push("20".to_string());
        args.push("-preset".to_string());
        args.push("medium".to_string());
    } else if cpu.libx264 {
        args.push("-c:v".to_string());
        args.push("libx264".to_string());
        args.push("-crf".to_string());
        args.push("18".to_string());
        args.push("-preset".to_string());
        args.push("medium".to_string());
    } else {
        // Bundled FFmpeg is built with --disable-libx264/--disable-libx265 (GPL); OpenH264
        // has no CRF mode, so use a high fixed bitrate for near-lossless output.
        args.push("-c:v".to_string());
        args.push("libopenh264".to_string());
        args.push("-b:v".to_string());
        args.push("20M".to_string());
        args.push("-pix_fmt".to_string());
        args.push("yuv420p".to_string());
    }
}

fn append_video_encode(
    args: &mut Vec<String>,
    output_ext: &str,
    hw: &HwEncoders,
    cpu: &CpuEncoders,
) {
    args.push("-threads".to_string());
    args.push("0".to_string());

    let output_ext = normalize_ext(output_ext);
    let use_hevc = output_ext == "mkv";

    if output_ext == "webm" {
        append_webm_video_codec(args, cpu);
    } else if hw.nvenc {
        args.push("-c:v".to_string());
        args.push(if use_hevc {
            "hevc_nvenc".to_string()
        } else {
            "h264_nvenc".to_string()
        });
        args.push("-cq".to_string());
        args.push("19".to_string());
        args.push("-preset".to_string());
        args.push("p5".to_string());
    } else if hw.amf {
        args.push("-c:v".to_string());
        args.push(if use_hevc {
            "hevc_amf".to_string()
        } else {
            "h264_amf".to_string()
        });
        args.push("-qp_i".to_string());
        args.push("19".to_string());
        args.push("-qp_p".to_string());
        args.push("19".to_string());
    } else if hw.qsv {
        args.push("-c:v".to_string());
        args.push(if use_hevc {
            "hevc_qsv".to_string()
        } else {
            "h264_qsv".to_string()
        });
        args.push("-global_quality".to_string());
        args.push("19".to_string());
    } else {
        append_software_video_codec(args, &output_ext, cpu);
    }
}

fn append_audio_encode(args: &mut Vec<String>, output_ext: &str) {
    let output_ext = normalize_ext(output_ext);
    match output_ext.as_str() {
        "mp3" => {
            args.push("-c:a".to_string());
            args.push("libmp3lame".to_string());
            args.push("-q:a".to_string());
            args.push("0".to_string());
        }
        "aac" | "m4a" => {
            args.push("-c:a".to_string());
            args.push("aac".to_string());
            args.push("-b:a".to_string());
            args.push("256k".to_string());
        }
        "flac" => {
            args.push("-c:a".to_string());
            args.push("flac".to_string());
        }
        "ogg" => {
            args.push("-c:a".to_string());
            args.push("libvorbis".to_string());
            args.push("-q:a".to_string());
            args.push("6".to_string());
        }
        "wav" => {
            args.push("-c:a".to_string());
            args.push("pcm_s16le".to_string());
        }
        _ => {
            args.push("-c:a".to_string());
            args.push("copy".to_string());
        }
    }
}

fn build_image_args(
    args: &mut Vec<String>,
    _input_ext: &str,
    output_ext: &str,
    is_lossless_source: bool,
    image_encoders: &ImageEncoders,
) {
    let output_ext = normalize_ext(output_ext);
    let lossless_target = is_lossless_extension(&output_ext)
        || (output_ext == "webp" && is_lossless_source);

    // Single-frame still image output (required for bmp/tiff/ico and many muxers).
    args.push("-frames:v".to_string());
    args.push("1".to_string());
    args.push("-update".to_string());
    args.push("1".to_string());

    match output_ext.as_str() {
        "ico" => {
            args.push("-vf".to_string());
            args.push(
                "scale=256:256:force_original_aspect_ratio=decrease,pad=256:256:(ow-iw)/2:(oh-ih)/2"
                    .to_string(),
            );
            args.push("-pix_fmt".to_string());
            args.push("bgra".to_string());
        }
        "bmp" => {
            args.push("-pix_fmt".to_string());
            args.push("bgr24".to_string());
        }
        "avif" => append_avif_encode(args, image_encoders),
        "gif" => {
            args.push("-loop".to_string());
            args.push("0".to_string());
        }
        "jpg" | "jpeg" => {
            args.push("-q:v".to_string());
            args.push(if lossless_target { "1" } else { "2" }.to_string());
        }
        "webp" => {
            if lossless_target {
                args.push("-lossless".to_string());
                args.push("1".to_string());
            } else {
                args.push("-q:v".to_string());
                args.push("2".to_string());
            }
        }
        "png" | "tiff" | "tif" => {
            if output_ext == "png" || output_ext == "tiff" || output_ext == "tif" {
                args.push("-compression_level".to_string());
                args.push("6".to_string());
            }
            args.push("-q:v".to_string());
            args.push("1".to_string());
        }
        _ => {
            if lossless_target {
                args.push("-q:v".to_string());
                args.push("1".to_string());
            } else {
                args.push("-q:v".to_string());
                args.push("2".to_string());
            }
        }
    }
}

fn append_avif_encode(args: &mut Vec<String>, image_encoders: &ImageEncoders) {
    let encoder = if image_encoders.svt_av1 {
        "libsvtav1"
    } else if image_encoders.libaom_av1 {
        "libaom-av1"
    } else {
        "libaom-av1"
    };
    args.push("-c:v".to_string());
    args.push(encoder.to_string());
    args.push("-still-picture".to_string());
    args.push("1".to_string());
    args.push("-crf".to_string());
    args.push("35".to_string());
}

fn build_video_to_gif(args: &mut Vec<String>) {
    args.push("-vf".to_string());
    args.push(
        "fps=15,scale=iw:-2:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer"
            .to_string(),
    );
    args.push("-loop".to_string());
    args.push("0".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_registry::ConversionTier;
    use crate::probe::ProbeResult;
    use crate::format_registry::MediaCategory;

    const NO_X264: CpuEncoders = CpuEncoders {
        libx264: false,
        libx265: false,
        libsvtav1: true,
        libvpx_vp9: true,
    };

    fn png_probe() -> ProbeResult {
        ProbeResult {
            category: MediaCategory::Image,
            video_codec: Some("png".to_string()),
            audio_codec: None,
            duration_secs: None,
            is_lossless_source: true,
            has_video: true,
            has_audio: false,
        }
    }

  #[test]
    fn ico_output_uses_bgra_and_single_frame_flags() {
        let args = build_ffmpeg_args(
            "in.png",
            "out.ico",
            &png_probe(),
            "png",
            "ico",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: false,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            false,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-update"));
        assert!(joined.contains("bgra"));
        assert!(joined.contains("256:256"));
    }

    #[test]
    fn avif_output_uses_still_picture_encoder() {
        let args = build_ffmpeg_args(
            "in.png",
            "out.avif",
            &png_probe(),
            "png",
            "avif",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: false,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            false,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-still-picture"));
        assert!(joined.contains("-crf"));
        assert!(joined.contains("av1") || joined.contains("svtav1"));
    }

    #[test]
    fn m3u8_input_adds_protocol_whitelist() {
        let probe = ProbeResult {
            category: MediaCategory::Video,
            video_codec: Some("h264".to_string()),
            audio_codec: Some("aac".to_string()),
            duration_secs: Some(3600.0),
            is_lossless_source: false,
            has_video: true,
            has_audio: true,
        };
        let args = build_ffmpeg_args(
            "https://cdn.example.com/vod/index.m3u8",
            "out.mp4",
            &probe,
            "m3u8",
            "mp4",
            ConversionTier::Copy,
            &HwEncoders {
                nvenc: false,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            false,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-protocol_whitelist"));
        assert!(joined.contains("crypto"));
        assert!(joined.contains("-reconnect"));
        assert!(joined.contains("-reconnect_streamed"));
        assert!(joined.contains("-fflags +genpts") || joined.contains("+genpts"));
        assert!(joined.contains("-avoid_negative_ts"));
        assert!(joined.contains("-bsf:a aac_adtstoasc"));
        assert!(!joined.contains("-hwaccel"));
    }

    #[test]
    fn local_hls_playlist_skips_http_reconnect_options() {
        let probe = ProbeResult {
            category: MediaCategory::Video,
            video_codec: Some("h264".to_string()),
            audio_codec: Some("aac".to_string()),
            duration_secs: Some(60.0),
            is_lossless_source: false,
            has_video: true,
            has_audio: true,
        };
        let args = build_ffmpeg_args(
            r"C:\Temp\file-converter-hls\range-test.m3u8",
            "out.mp4",
            &probe,
            "m3u8",
            "mp4",
            ConversionTier::Copy,
            &HwEncoders {
                nvenc: false,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            false,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-protocol_whitelist"));
        assert!(joined.contains("+genpts"));
        assert!(!joined.contains("-reconnect"));
        assert!(joined.contains("-avoid_negative_ts"));
    }

    #[test]
    fn m3u8_encode_skips_hwaccel() {
        let probe = ProbeResult {
            category: MediaCategory::Video,
            video_codec: Some("hevc".to_string()),
            audio_codec: Some("aac".to_string()),
            duration_secs: Some(3600.0),
            is_lossless_source: false,
            has_video: true,
            has_audio: true,
        };
        let args = build_ffmpeg_args(
            "https://cdn.example.com/vod/index.m3u8",
            "out.mkv",
            &probe,
            "m3u8",
            "mkv",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: true,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            false,
        );
        let joined = args.join(" ");
        assert!(!joined.contains("-hwaccel"));
    }

    fn prores_probe() -> ProbeResult {
        ProbeResult {
            category: MediaCategory::Video,
            video_codec: Some("prores".to_string()),
            audio_codec: Some("pcm_s24le".to_string()),
            duration_secs: Some(4.5),
            is_lossless_source: true,
            has_video: true,
            has_audio: true,
        }
    }

    #[test]
    fn prores_never_uses_hw_decode() {
        let args = build_ffmpeg_args(
            "in.mov",
            "out.mp4",
            &prores_probe(),
            "mov",
            "mp4",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: true,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            false,
        );
        assert!(!args.iter().any(|a| a == "-hwaccel"), "{args:?}");
        assert!(args.join(" ").contains("-c:v h264_nvenc"));
    }

    #[test]
    fn hwaccel_is_an_input_option_before_dash_i() {
        let mut probe = prores_probe();
        probe.video_codec = Some("hevc".to_string());
        let args = build_ffmpeg_args(
            "in.mov",
            "out.mp4",
            &probe,
            "mov",
            "mp4",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: true,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            false,
        );
        let hwaccel = args.iter().position(|a| a == "-hwaccel").expect("hwaccel present");
        let input = args.iter().position(|a| a == "-i").expect("-i present");
        assert!(hwaccel < input, "{args:?}");
        assert_eq!(args[hwaccel + 1], "d3d11va");
        assert!(!args.iter().any(|a| a == "-hwaccel_output_format"));
    }

    #[test]
    fn cpu_fallback_uses_openh264_when_x264_missing() {
        let args = build_ffmpeg_args(
            "in.mov",
            "out.mp4",
            &prores_probe(),
            "mov",
            "mp4",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: true,
                amf: false,
                qsv: false,
            },
            &NO_X264,
            true,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-c:v libopenh264"), "{joined}");
        assert!(!joined.contains("libx264"));
        assert!(!joined.contains("-hwaccel"));
    }

    #[test]
    fn cpu_fallback_prefers_x264_when_available() {
        let args = build_ffmpeg_args(
            "in.mov",
            "out.mkv",
            &prores_probe(),
            "mov",
            "mkv",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: false,
                amf: false,
                qsv: false,
            },
            &CpuEncoders {
                libx264: true,
                libx265: true,
                libsvtav1: false,
                libvpx_vp9: false,
            },
            true,
        );
        assert!(args.join(" ").contains("-c:v libx265"));
    }

    #[test]
    fn mp3_to_ogg_does_not_enable_hwaccel() {
        let probe = ProbeResult {
            category: MediaCategory::Audio,
            video_codec: None,
            audio_codec: Some("mp3".to_string()),
            duration_secs: Some(180.0),
            is_lossless_source: false,
            has_video: false,
            has_audio: true,
        };
        let args = build_ffmpeg_args(
            "in.mp3",
            "out.ogg",
            &probe,
            "mp3",
            "ogg",
            ConversionTier::EncodeHq,
            &HwEncoders {
                nvenc: true,
                amf: true,
                qsv: true,
            },
            &NO_X264,
            false,
        );
        let joined = args.join(" ");
        assert!(!joined.contains("-hwaccel"));
    }

    #[test]
    fn webm_never_uses_h264_encoders() {
        let all_hw = HwEncoders {
            nvenc: true,
            amf: true,
            qsv: true,
        };
        for cpu_only in [false, true] {
            let args = build_ffmpeg_args(
                "in.mov",
                "out.webm",
                &prores_probe(),
                "mov",
                "webm",
                ConversionTier::EncodeHq,
                &all_hw,
                &NO_X264,
                cpu_only,
            );
            let joined = args.join(" ");
            assert!(joined.contains("-c:v libsvtav1"), "{joined}");
            assert!(!joined.contains("264") && !joined.contains("hevc"), "{joined}");
        }
        let vp9_only = CpuEncoders {
            libx264: false,
            libx265: false,
            libsvtav1: false,
            libvpx_vp9: true,
        };
        let args = build_ffmpeg_args(
            "in.mov",
            "out.webm",
            &prores_probe(),
            "mov",
            "webm",
            ConversionTier::EncodeHq,
            &all_hw,
            &vp9_only,
            false,
        );
        assert!(args.join(" ").contains("-c:v libvpx-vp9"));
    }
}
