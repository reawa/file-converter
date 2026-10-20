export type MediaCategory = "video" | "audio" | "image" | "unknown";
export type ConversionTier = "COPY" | "ENCODE_HQ";

export interface FormatGroup {
  label: string;
  formats: string[];
}

export interface YouTubeQuality {
  label: string;
  value: string;
}

export interface FileJobInfo {
  id: string;
  path: string;
  name: string;
  category: MediaCategory;
  tier: ConversionTier;
  output_format: string;
  hls_start_secs?: number | null;
  hls_end_secs?: number | null;
  youtube_quality?: string | null;
  youtube_qualities?: YouTubeQuality[] | null;
  platform?: string | null;
  error: string | null;
}

export interface ConversionJob {
  id: string;
  path: string;
  output_format: string;
  hls_start_secs?: number | null;
  hls_end_secs?: number | null;
  youtube_quality?: string | null;
  output_name?: string | null;
}

export interface ProgressEvent {
  id: string;
  progress: number;
  status: string;
  elapsed_secs: number;
  /** Seconds remaining, estimated from the progress rate; null until it can be measured. */
  eta_secs: number | null;
}

export interface CompleteEvent {
  id: string;
  output_path: string;
  elapsed_secs: number;
}

/** Format seconds as `45s`, `3m 07s`, or `1h 02m 09s`. */
export function formatElapsed(totalSecs: number): string {
  const secs = Math.max(0, Math.round(totalSecs));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  if (h > 0) return `${h}h ${pad(m)}m ${pad(s)}s`;
  if (m > 0) return `${m}m ${pad(s)}s`;
  return `${s}s`;
}

export interface ErrorEvent {
  id: string;
  message: string;
}

export interface CancelledEvent {
  id: string;
}

export interface SkippedEvent {
  id: string;
  message: string;
}

export interface OutputConflict {
  id: string;
  output_path: string;
}

export interface JobTiming {
  /** Epoch ms the job started (backend elapsed time, adjusted to the local clock). */
  startedAt: number;
  etaSecs: number | null;
  /** Epoch ms the ETA was measured, so the remaining time can count down between events. */
  etaAt: number;
}

export interface QueuedFile extends FileJobInfo {
  progress: number;
  status: string;
  done: boolean;
  cancelled?: boolean;
  skipped?: boolean;
  output_path?: string;
  /** Live timing while converting; cleared when the job finishes or is reset. */
  timing?: JobTiming;
  /** Custom output stem (no extension). When unset, derived from name/path. */
  output_name?: string | null;
}

export const CATEGORY_LABEL: Record<MediaCategory, string> = {
  video: "Video",
  audio: "Audio",
  image: "Image",
  unknown: "Unknown",
};

export function truncateError(msg: string, max = 48): string {
  if (msg.length <= max) return msg;
  return `${msg.slice(0, max - 1)}…`;
}

export function isHlsSource(path: string): boolean {
  const trimmed = path.trim().toLowerCase();
  if (
    (trimmed.startsWith("http://") || trimmed.startsWith("https://")) &&
    trimmed.includes(".m3u8")
  ) {
    return true;
  }
  return trimmed.endsWith(".m3u8");
}

export function isYoutubeSource(path: string): boolean {
  const trimmed = path.trim().toLowerCase();
  if (!(trimmed.startsWith("http://") || trimmed.startsWith("https://"))) {
    return false;
  }
  return (
    trimmed.includes("youtube.com/") ||
    trimmed.includes("youtu.be/") ||
    trimmed.includes("music.youtube.com/")
  );
}

/** Any http(s) link that is not a raw HLS playlist — handled via yt-dlp. */
export function isRemoteSource(path: string): boolean {
  const trimmed = path.trim().toLowerCase();
  if (!(trimmed.startsWith("http://") || trimmed.startsWith("https://"))) {
    return false;
  }
  return !trimmed.includes(".m3u8");
}

export function isSupportedPasteUrl(path: string): boolean {
  return isHlsSource(path) || isRemoteSource(path);
}
