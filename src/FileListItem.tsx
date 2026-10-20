import { memo, useEffect, useMemo, useState } from "react";
import type { FormatGroup, QueuedFile } from "./types";
import {
  CATEGORY_LABEL,
  formatElapsed,
  isHlsSource,
  isRemoteSource,
  truncateError,
} from "./types";
import { formatHlsTime, parseHlsTimeInput } from "./hlsTime";
import { FormatSelect } from "./FormatSelect";
import {
  AudioIcon,
  ExternalOpenIcon,
  FileEmptyIcon,
  FolderOpenIcon,
  ImageIcon,
  VideoIcon,
  XIcon,
} from "./icons";

interface FileListItemProps {
  file: QueuedFile;
  formatGroups: FormatGroup[];
  disabled?: boolean;
  onRemove: (id: string) => void;
  onFormatChange: (id: string, format: string) => void;
  onHlsRangeChange: (id: string, start: string, end: string) => void;
  onHlsDraftChange?: (id: string, start: string, end: string) => void;
  onYoutubeQualityChange: (id: string, quality: string) => void;
  onRename: (id: string, name: string) => void;
  onRevealInExplorer: (path: string) => void;
  onOpenFile: (path: string) => void;
}

function TypeIcon({ category }: { category: string }) {
  const cls = `file-type-icon ${category}`;
  switch (category) {
    case "video":
      return <div className={cls}><VideoIcon /></div>;
    case "audio":
      return <div className={cls}><AudioIcon /></div>;
    case "image":
      return <div className={cls}><ImageIcon /></div>;
    default:
      return <div className="file-type-icon unknown"><FileEmptyIcon /></div>;
  }
}

/** Current time, re-rendered every second while `active`. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

function FileListItemInner({
  file,
  formatGroups,
  disabled,
  onRemove,
  onFormatChange,
  onHlsRangeChange,
  onHlsDraftChange,
  onYoutubeQualityChange,
  onRename,
  onRevealInExplorer,
  onOpenFile,
}: FileListItemProps) {
  const isHls = isHlsSource(file.path);
  const isRemote = isRemoteSource(file.path);
  const platformLabel = file.platform?.trim() || (isRemote ? "Link" : null);
  const displayName = file.output_name?.trim() || file.name;
  const [nameInput, setNameInput] = useState(displayName);
  const [startInput, setStartInput] = useState(formatHlsTime(file.hls_start_secs));
  const [endInput, setEndInput] = useState(formatHlsTime(file.hls_end_secs));

  useEffect(() => {
    setNameInput(file.output_name?.trim() || file.name);
  }, [file.name, file.output_name]);

  useEffect(() => {
    setStartInput(formatHlsTime(file.hls_start_secs));
    setEndInput(formatHlsTime(file.hls_end_secs));
  }, [file.hls_start_secs, file.hls_end_secs]);

  function commitRename() {
    const next = nameInput.trim();
    if (!next) {
      setNameInput(displayName);
      return;
    }
    if (next !== displayName) {
      onRename(file.id, next);
    }
  }

  const timing = file.timing;
  const now = useNow(!!timing);
  let timingText: string | null = null;
  if (timing) {
    const elapsed = Math.max(0, (now - timing.startedAt) / 1000);
    timingText = `${formatElapsed(elapsed)} elapsed`;
    if (timing.etaSecs !== null) {
      const left = Math.max(0, timing.etaSecs - (now - timing.etaAt) / 1000);
      timingText += ` · ~${formatElapsed(left)} left`;
    }
  }

  const qualityLockedAudio = file.youtube_quality === "audio";
  const effectiveGroups = useMemo(() => {
    if (!isRemote || !qualityLockedAudio) return formatGroups;
    return formatGroups
      .map((g) => ({
        ...g,
        formats: g.formats.filter((f) =>
          ["mp3", "m4a", "aac", "opus", "ogg", "flac", "wav"].includes(f)
        ),
      }))
      .filter((g) => g.formats.length > 0);
  }, [formatGroups, isRemote, qualityLockedAudio]);

  const statusBadge = file.cancelled
    ? <span className="badge cancelled">CANCELLED</span>
    : file.skipped
      ? <span className="badge skipped">SKIPPED</span>
      : file.error
        ? (
          <span className="badge error" title={file.error}>
            {truncateError(file.error)}
          </span>
        )
        : (
          <span className={`badge ${file.tier === "COPY" ? "copy" : "encode"}`}>
            {file.tier === "COPY" ? "COPY" : "ENCODE HQ"}
          </span>
        );

  return (
    <li className="file-item">
      <div className="file-row">
        <TypeIcon category={file.category} />
        <div className="file-body">
          <input
            type="text"
            className="file-name-input"
            value={nameInput}
            disabled={disabled || file.done || file.cancelled || file.skipped}
            aria-label={`Output name for ${file.name}`}
            title="Click to rename output file"
            onChange={(e) => setNameInput(e.target.value)}
            onBlur={commitRename}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.currentTarget.blur();
              }
              if (e.key === "Escape") {
                setNameInput(displayName);
                e.currentTarget.blur();
              }
            }}
          />
          <div className="file-meta">
            <span className="category-tag">{CATEGORY_LABEL[file.category]}</span>
            {isHls && <span className="category-tag hls">HLS</span>}
            {platformLabel && (
              <span className="category-tag youtube">{platformLabel}</span>
            )}
            {statusBadge}
          </div>
          {isHls && (
            <div className="hls-range-row">
              <label className="hls-range-field">
                <span>Start</span>
                <input
                  type="text"
                  className="hls-range-input"
                  placeholder="0:00:00"
                  value={startInput}
                  disabled={disabled || file.done || file.cancelled || file.skipped}
                  onChange={(e) => {
                    const value = e.target.value;
                    setStartInput(value);
                    onHlsDraftChange?.(
                      file.id,
                      parseHlsTimeInput(value),
                      parseHlsTimeInput(endInput)
                    );
                  }}
                  onBlur={() =>
                    onHlsRangeChange(
                      file.id,
                      parseHlsTimeInput(startInput),
                      parseHlsTimeInput(endInput)
                    )
                  }
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.currentTarget.blur();
                    }
                  }}
                />
              </label>
              <label className="hls-range-field">
                <span>End</span>
                <input
                  type="text"
                  className="hls-range-input"
                  placeholder="optional"
                  value={endInput}
                  disabled={disabled || file.done || file.cancelled || file.skipped}
                  onChange={(e) => {
                    const value = e.target.value;
                    setEndInput(value);
                    onHlsDraftChange?.(
                      file.id,
                      parseHlsTimeInput(startInput),
                      parseHlsTimeInput(value)
                    );
                  }}
                  onBlur={() =>
                    onHlsRangeChange(
                      file.id,
                      parseHlsTimeInput(startInput),
                      parseHlsTimeInput(endInput)
                    )
                  }
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.currentTarget.blur();
                    }
                  }}
                />
              </label>
            </div>
          )}
          {isRemote && (file.youtube_qualities?.length ?? 0) > 0 && (
            <div className="youtube-quality-row">
              <label className="youtube-quality-field">
                <span>Quality</span>
                <select
                  className="youtube-quality-select"
                  value={file.youtube_quality ?? "best"}
                  disabled={disabled || file.done || file.cancelled || file.skipped || !!file.error}
                  aria-label={`Quality for ${file.name}`}
                  onChange={(e) => onYoutubeQualityChange(file.id, e.target.value)}
                >
                  {file.youtube_qualities!.map((q) => (
                    <option key={q.value} value={q.value}>
                      {q.label}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          )}
        </div>
        <div className="file-actions">
          {file.done && file.output_path && (
            <>
              <button
                type="button"
                className="btn-icon btn-icon-action"
                title="Open file"
                onClick={() => onOpenFile(file.output_path!)}
              >
                <ExternalOpenIcon />
              </button>
              <button
                type="button"
                className="btn-icon btn-icon-action"
                title="Show in File Explorer"
                onClick={() => onRevealInExplorer(file.output_path!)}
              >
                <FolderOpenIcon />
              </button>
            </>
          )}
          <FormatSelect
            value={file.output_format}
            groups={effectiveGroups}
            disabled={disabled || file.error !== null || file.done || file.cancelled || file.skipped}
            ariaLabel={`Output format for ${file.name}`}
            onChange={(fmt) => onFormatChange(file.id, fmt)}
          />
          <button
            type="button"
            className="btn-icon btn-icon-remove"
            disabled={disabled}
            onClick={() => onRemove(file.id)}
            aria-label={`Remove ${file.name}`}
          >
            <XIcon />
          </button>
        </div>
      </div>
      {(file.progress > 0 || file.status) && (
        <div className="progress-wrap">
          <div
            className="progress-bar"
            style={{ width: `${file.progress}%` }}
          />
          <span className="progress-text" title={file.status}>
            <span className="progress-status">{file.status}</span>
            {timingText && <span className="progress-timing">{timingText}</span>}
          </span>
        </div>
      )}
    </li>
  );
}

export const FileListItem = memo(FileListItemInner);
