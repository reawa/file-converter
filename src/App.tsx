import { useEffect, useState, useCallback, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { DropZone } from "./DropZone";
import { FileList } from "./FileList";
import { TitleBar } from "./TitleBar";
import { FolderOpenIcon, RefreshIcon, XIcon } from "./icons";
import "./App.css";
import type {
  CancelledEvent,
  CompleteEvent,
  ConversionJob,
  ErrorEvent,
  FileJobInfo,
  OutputConflict,
  ProgressEvent,
  QueuedFile,
  SkippedEvent,
} from "./types";
import { formatElapsed, isHlsSource, isRemoteSource, isSupportedPasteUrl } from "./types";
import {
  checkForUpdate,
  installPendingUpdate,
  isTauri,
} from "./lib/updater";

const AUTO_UPDATE_KEY = "file-converter.autoUpdate";

function toQueued(info: FileJobInfo): QueuedFile {
  return {
    ...info,
    progress: 0,
    status: "",
    done: false,
  };
}

function normalizePath(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
}

function useUpdatePrompt(): {
  version: string | null;
  busy: boolean;
  install: () => void;
  dismiss: () => void;
} {
  // Opt-in only: set localStorage file-converter.autoUpdate = "true" to install without a click.
  const auto =
    typeof localStorage !== "undefined" &&
    localStorage.getItem(AUTO_UPDATE_KEY) === "true";
  const [version, setVersion] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    const t = window.setTimeout(() => {
      void (async () => {
        const result = await checkForUpdate();
        if (result.kind !== "available") return;
        setVersion(result.version);
        if (!auto) return;
        setBusy(true);
        const installed = await installPendingUpdate();
        if (installed !== "updated") setBusy(false);
        if (installed === "error" || installed === "none") setVersion(null);
      })();
    }, 1500);
    return () => window.clearTimeout(t);
  }, [auto]);

  const install = () => {
    if (busy) return;
    setBusy(true);
    void installPendingUpdate().then((r) => {
      if (r !== "updated") setBusy(false);
      if (r === "error" || r === "none") setVersion(null);
    });
  };

  return { version, busy, install, dismiss: () => setVersion(null) };
}

export default function App() {
  const [files, setFiles] = useState<QueuedFile[]>([]);
  const [outputDir, setOutputDir] = useState<string | null>(null);
  const [defaultDownloadsDir, setDefaultDownloadsDir] = useState<string | null>(null);
  const [converting, setConverting] = useState(false);
  const [isProbing, setIsProbing] = useState(false);
  const [dragActive, setDragActive] = useState(false);
  const [ffmpegError, setFfmpegError] = useState<string | null>(null);
  const [conversionError, setConversionError] = useState<string | null>(null);
  const [supportedExtensions, setSupportedExtensions] = useState<string[]>([]);

  const filesRef = useRef(files);
  filesRef.current = files;
  const convertingRef = useRef(converting);
  convertingRef.current = converting;
  const isProbingRef = useRef(isProbing);
  isProbingRef.current = isProbing;
  const outputDirRef = useRef(outputDir);
  outputDirRef.current = outputDir;
  const ffmpegOkRef = useRef<boolean | null>(null);
  const formatChangeRef = useRef<string | null>(null);
  const hlsDraftRef = useRef<Record<string, { start: string; end: string }>>({});
  const updatePrompt = useUpdatePrompt();

  const validCount = useMemo(
    () => files.filter((f) => !f.error && !f.done).length,
    [files]
  );
  const doneCount = useMemo(() => files.filter((f) => f.done).length, [files]);

  const addPaths = useCallback(async (paths: string[]) => {
    if (paths.length === 0 || convertingRef.current || isProbingRef.current) return;
    if (ffmpegOkRef.current === false) return;

    const existing = new Set(
      filesRef.current.map((f) => normalizePath(f.path))
    );
    const unique = paths.filter((p) => !existing.has(normalizePath(p)));
    if (unique.length === 0) return;

    isProbingRef.current = true;
    setIsProbing(true);
    const batchPaths = new Set(unique.map(normalizePath));
    try {
      const newJobs = await invoke<FileJobInfo[]>("probe_files", {
        paths: unique,
        outputFormat: null,
      });
      setFiles((prev) => {
        const seen = new Set(prev.map((f) => normalizePath(f.path)));
        const toAdd = newJobs
          .filter(
            (job) =>
              batchPaths.has(normalizePath(job.path)) &&
              !seen.has(normalizePath(job.path))
          )
          .map(toQueued);
        return toAdd.length === 0 ? prev : [...prev, ...toAdd];
      });
      ffmpegOkRef.current = true;
    } catch (e) {
      const msg = String(e);
      if (msg.toLowerCase().includes("ffmpeg") || msg.toLowerCase().includes("ffprobe")) {
        ffmpegOkRef.current = false;
        setFfmpegError(msg);
      }
      setConversionError(msg);
    } finally {
      isProbingRef.current = false;
      setIsProbing(false);
    }
  }, []);

  useEffect(() => {
    void invoke<string>("get_default_output_dir")
      .then((dir) => setDefaultDownloadsDir(dir))
      .catch(() => setDefaultDownloadsDir(null));
  }, []);

  useEffect(() => {
    function onPaste(e: ClipboardEvent) {
      if (convertingRef.current || isProbingRef.current) return;
      const text = e.clipboardData?.getData("text")?.trim();
      if (!text) return;
      const lower = text.toLowerCase();
      if (!(lower.startsWith("http://") || lower.startsWith("https://"))) return;

      e.preventDefault();
      if (isSupportedPasteUrl(text)) {
        void addPaths([text]);
      } else {
        setConversionError(
          "This link type is not supported yet. Paste an http(s) media link or HLS (.m3u8) URL."
        );
      }
    }

    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  }, [addPaths]);

  useEffect(() => {
    let disposed = false;
    let unlistenDrag: (() => void) | undefined;

    getCurrentWindow()
      .onDragDropEvent((event) => {
        if (convertingRef.current || isProbingRef.current) return;

        const { type } = event.payload;
        if (type === "enter" || type === "over") {
          setDragActive(true);
        } else if (type === "leave") {
          setDragActive(false);
        } else if (type === "drop") {
          setDragActive(false);
          addPaths(event.payload.paths);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlistenDrag = fn;
      });

    return () => {
      disposed = true;
      unlistenDrag?.();
    };
  }, [addPaths]);

  useEffect(() => {
    const unlistenProgress = listen<ProgressEvent>("conversion-progress", (e) => {
      setFiles((prev) =>
        prev.map((f) =>
          f.id === e.payload.id
            ? {
                ...f,
                progress: e.payload.progress,
                status: e.payload.status,
                timing: {
                  startedAt: Date.now() - e.payload.elapsed_secs * 1000,
                  etaSecs: e.payload.eta_secs,
                  etaAt: Date.now(),
                },
              }
            : f
        )
      );
    });

    const unlistenComplete = listen<CompleteEvent>("conversion-complete", (e) => {
      setFiles((prev) =>
        prev.map((f) =>
          f.id === e.payload.id
            ? {
                ...f,
                progress: 100,
                status: `Done in ${formatElapsed(e.payload.elapsed_secs)} · Saved: ${e.payload.output_path}`,
                done: true,
                output_path: e.payload.output_path,
                timing: undefined,
              }
            : f
        )
      );
    });

    const unlistenError = listen<ErrorEvent>("conversion-error", (e) => {
      if (!e.payload.id) {
        setConversionError(e.payload.message);
        setConverting(false);
        return;
      }
      setFiles((prev) =>
        prev.map((f) =>
          f.id === e.payload.id
            ? { ...f, status: e.payload.message, error: e.payload.message, timing: undefined }
            : f
        )
      );
    });

    const unlistenCancelled = listen<CancelledEvent>("conversion-cancelled", (e) => {
      setFiles((prev) =>
        prev.map((f) =>
          f.id === e.payload.id
            ? {
                ...f,
                cancelled: true,
                status: "Cancelled",
                progress: 0,
                timing: undefined,
              }
            : f
        )
      );
    });

    const unlistenSkipped = listen<SkippedEvent>("conversion-skipped", (e) => {
      setFiles((prev) =>
        prev.map((f) =>
          f.id === e.payload.id
            ? {
                ...f,
                skipped: true,
                status: e.payload.message,
                progress: 0,
                timing: undefined,
              }
            : f
        )
      );
    });

    const unlistenQueue = listen("queue-complete", () => {
      setConverting(false);
    });

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
      unlistenError.then((fn) => fn());
      unlistenCancelled.then((fn) => fn());
      unlistenSkipped.then((fn) => fn());
      unlistenQueue.then((fn) => fn());
    };
  }, []);

  function removeFile(id: string) {
    setFiles((prev) => prev.filter((f) => f.id !== id));
  }

  function clearQueue() {
    if (converting) return;
    setFiles([]);
  }

  function clearCompleted() {
    if (converting) return;
    setFiles((prev) => prev.filter((f) => !f.done));
  }

  async function changeFormat(id: string, format: string) {
    const file = filesRef.current.find((f) => f.id === id);
    if (!file || convertingRef.current || format === file.output_format) return;
    if (formatChangeRef.current === id) return;

    formatChangeRef.current = id;
    try {
      const updated = await invoke<FileJobInfo>("update_job_format", {
        path: file.path,
        outputFormat: format,
        youtubeQuality: file.youtube_quality ?? null,
      });
      setFiles((prev) =>
        prev.map((f) =>
          f.id === id
            ? {
                ...toQueued({ ...updated, id: f.id }),
                hls_start_secs: updated.hls_start_secs ?? f.hls_start_secs,
                hls_end_secs: updated.hls_end_secs ?? f.hls_end_secs,
                youtube_quality: updated.youtube_quality ?? f.youtube_quality,
                youtube_qualities: updated.youtube_qualities ?? f.youtube_qualities,
                platform: updated.platform ?? f.platform,
                output_name: f.output_name,
                progress: 0,
                status: "",
                done: false,
                cancelled: false,
                skipped: false,
                error: null,
                output_path: undefined,
                timing: undefined,
              }
            : f
        )
      );
    } catch (e) {
      setFiles((prev) =>
        prev.map((f) =>
          f.id === id ? { ...f, error: String(e) } : f
        )
      );
    } finally {
      formatChangeRef.current = null;
    }
  }

  function noteHlsDraft(id: string, start: string, end: string) {
    hlsDraftRef.current[id] = { start, end };
  }

  async function changeHlsRange(id: string, start: string, end: string) {
    const file = filesRef.current.find((f) => f.id === id);
    if (!file || convertingRef.current || !isHlsSource(file.path)) return;

    hlsDraftRef.current[id] = { start, end };

    try {
      const updated = await invoke<FileJobInfo>("update_job_hls_range", {
        path: file.path,
        outputFormat: file.output_format,
        hlsStart: start || null,
        hlsEnd: end || null,
      });
      setFiles((prev) =>
        prev.map((f) =>
            f.id === id
            ? {
                ...toQueued({ ...updated, id: f.id }),
                error: null,
                progress: f.progress,
                status: f.status,
                done: f.done,
                cancelled: f.cancelled,
                skipped: f.skipped,
                output_path: f.output_path,
                output_name: f.output_name,
              }
            : f
        )
      );
      delete hlsDraftRef.current[id];
    } catch (e) {
      setFiles((prev) =>
        prev.map((f) => (f.id === id ? { ...f, error: String(e) } : f))
      );
    }
  }

  /** Commit any unblurred HLS Start/End values before building conversion jobs. */
  async function flushPendingHlsRanges(
    filesToFlush: QueuedFile[]
  ): Promise<QueuedFile[]> {
    let next = [...filesToFlush];
    for (let i = 0; i < next.length; i++) {
      const file = next[i];
      if (!isHlsSource(file.path) || file.error || file.done) continue;
      const draft = hlsDraftRef.current[file.id];
      if (!draft) continue;
      try {
        const updated = await invoke<FileJobInfo>("update_job_hls_range", {
          path: file.path,
          outputFormat: file.output_format,
          hlsStart: draft.start || null,
          hlsEnd: draft.end || null,
        });
        next[i] = {
          ...toQueued({ ...updated, id: file.id }),
          error: null,
          progress: file.progress,
          status: file.status,
          done: file.done,
          cancelled: file.cancelled,
          skipped: file.skipped,
          output_path: file.output_path,
          output_name: file.output_name,
        };
        delete hlsDraftRef.current[file.id];
      } catch (e) {
        next[i] = { ...file, error: String(e) };
      }
    }
    setFiles((prev) => {
      const byId = new Map(next.map((f) => [f.id, f]));
      return prev.map((f) => byId.get(f.id) ?? f);
    });
    filesRef.current = filesRef.current.map((f) => {
      const updated = next.find((n) => n.id === f.id);
      return updated ?? f;
    });
    return next;
  }

  async function changeYoutubeQuality(id: string, quality: string) {
    const file = filesRef.current.find((f) => f.id === id);
    if (!file || convertingRef.current || !isRemoteSource(file.path)) return;
    if (quality === (file.youtube_quality ?? "best")) return;

    try {
      const updated = await invoke<FileJobInfo>("update_job_youtube_quality", {
        path: file.path,
        outputFormat: file.output_format,
        youtubeQuality: quality,
      });
      setFiles((prev) =>
        prev.map((f) =>
          f.id === id
            ? {
                ...toQueued({ ...updated, id: f.id }),
                error: null,
                progress: 0,
                status: "",
                done: false,
                cancelled: false,
                skipped: false,
                output_path: undefined,
                output_name: f.output_name,
              }
            : f
        )
      );
    } catch (e) {
      setFiles((prev) =>
        prev.map((f) => (f.id === id ? { ...f, error: String(e) } : f))
      );
    }
  }

  function renameFile(id: string, name: string) {
    if (convertingRef.current) return;
    const trimmed = name.trim();
    if (!trimmed) return;
    setFiles((prev) =>
      prev.map((f) =>
        f.id === id
          ? {
              ...f,
              output_name: trimmed,
              done: false,
              cancelled: false,
              skipped: false,
              output_path: undefined,
              timing: undefined,
              status: f.done ? "" : f.status,
            }
          : f
      )
    );
  }

  async function pickOutputDir() {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") setOutputDir(dir);
  }

  async function revealOutputInExplorer(filePath: string) {
    try {
      await invoke("reveal_output_path", { path: filePath });
    } catch (e) {
      setConversionError(String(e));
    }
  }

  async function openOutputFile(filePath: string) {
    try {
      await invoke("open_output_path", { path: filePath });
    } catch (e) {
      setConversionError(String(e));
    }
  }

  async function startConversion() {
    setConversionError(null);

    const flushed = await flushPendingHlsRanges(filesRef.current);
    const valid = flushed.filter((f) => !f.error && !f.done);
    if (valid.length === 0) {
      const firstErr = flushed.find((f) => f.error)?.error;
      if (firstErr) setConversionError(firstErr);
      return;
    }

    const jobs: ConversionJob[] = valid.map((f) => ({
      id: f.id,
      path: f.path,
      output_format: f.output_format,
      hls_start_secs: f.hls_start_secs ?? null,
      hls_end_secs: f.hls_end_secs ?? null,
      youtube_quality: f.youtube_quality ?? null,
      output_name: f.output_name?.trim() || null,
    }));

    try {
      const conflicts = await invoke<OutputConflict[]>("check_output_conflicts", {
        jobs,
        outputDir: outputDirRef.current,
      });

      let allowOverwrite = false;
      if (conflicts.length > 0) {
        const ok = await confirm(
          `${conflicts.length} output file(s) already exist and will be overwritten. Continue?`,
          { title: "Overwrite files?", kind: "warning" }
        );
        if (!ok) return;
        allowOverwrite = true;
      }

      setConverting(true);
      setFiles((prev) =>
        prev.map((f) =>
          f.error || f.done
            ? f
            : {
                ...f,
                progress: 0,
                status: "",
                cancelled: false,
                skipped: false,
                error: null,
                timing: undefined,
              }
        )
      );

      await invoke("start_conversion", {
        jobs,
        outputDir: outputDirRef.current,
        allowOverwrite,
      });
    } catch (e) {
      setConverting(false);
      setConversionError(String(e));
    }
  }

  async function cancelConversion() {
    try {
      await invoke("cancel_conversion");
    } catch (e) {
      setConversionError(String(e));
    }
  }

  async function pickFilesViaDialog() {
    if (converting || isProbing) return;
    if (supportedExtensions.length === 0) {
      try {
        const exts = await invoke<string[]>("get_supported_extensions");
        setSupportedExtensions(exts);
      } catch {
        /* dialog still works without filters */
      }
    }
    const selected = await open({
      multiple: true,
      directory: false,
      filters:
        supportedExtensions.length > 0
          ? [{ name: "Media files", extensions: supportedExtensions }]
          : undefined,
    });
    if (selected) {
      const paths = Array.isArray(selected) ? selected : [selected];
      await addPaths(paths);
    }
  }

  return (
    <div className="app-shell">
      <TitleBar />
      <main className="app" aria-busy={converting || isProbing}>
        {ffmpegError && (
          <div className="alert error">
            FFmpeg is required for conversion. Reinstall the app or install FFmpeg on
            your system PATH.
            <br />
            {ffmpegError}
          </div>
        )}

        {conversionError && !ffmpegError && (
          <div className="alert error">
            {conversionError}
            <button
              type="button"
              className="btn-text alert-dismiss"
              onClick={() => setConversionError(null)}
            >
              Dismiss
            </button>
          </div>
        )}

        <section className="panel">
          <DropZone
            onBrowse={pickFilesViaDialog}
            disabled={converting || isProbing}
            dragActive={dragActive}
            isProbing={isProbing}
          />
        </section>

        <section className="panel panel-files">
          <div className="panel-files-header">
            <span className="panel-label">Queue</span>
            <div className="panel-files-actions">
              {files.length > 0 && (
                <span className="file-count">
                  {files.length} file{files.length !== 1 ? "s" : ""}
                </span>
              )}
              {doneCount > 0 && !converting && (
                <button type="button" className="btn-text" onClick={clearCompleted}>
                  Clear completed
                </button>
              )}
              {files.length > 0 && !converting && (
                <button type="button" className="btn-text" onClick={clearQueue}>
                  Clear all
                </button>
              )}
            </div>
          </div>
          <FileList
            files={files}
            onRemove={removeFile}
            onFormatChange={changeFormat}
            onHlsRangeChange={changeHlsRange}
            onHlsDraftChange={noteHlsDraft}
            onYoutubeQualityChange={changeYoutubeQuality}
            onRename={renameFile}
            onRevealInExplorer={revealOutputInExplorer}
            onOpenFile={openOutputFile}
            disabled={converting}
          />
        </section>

        <footer className="panel action-bar">
          <div className="output-row">
            <button
              type="button"
              className="btn secondary"
              disabled={converting}
              onClick={pickOutputDir}
            >
              <FolderOpenIcon />
              {outputDir ? "Change folder" : "Default folders"}
            </button>
            {outputDir && (
              <button
                type="button"
                className="btn secondary"
                disabled={converting}
                onClick={() => setOutputDir(null)}
              >
                Use defaults
              </button>
            )}
          </div>
          <span
            className="output-dir"
            title={
              outputDir ??
              (defaultDownloadsDir
                ? `Links → ${defaultDownloadsDir} · Local files → same folder`
                : "Links → Downloads · Local files → same folder")
            }
          >
            {outputDir
              ? outputDir
              : defaultDownloadsDir
                ? `Links → Downloads · Files → source folder`
                : "Links → Downloads · Files → source folder"}
          </span>
          <div className="action-buttons">
            {converting ? (
              <button type="button" className="btn danger" onClick={cancelConversion}>
                Cancel
              </button>
            ) : (
              <button
                type="button"
                className="btn primary"
                disabled={validCount === 0 || ffmpegError !== null}
                onClick={startConversion}
              >
                Convert{validCount > 0 ? ` (${validCount})` : ""}
              </button>
            )}
          </div>
        </footer>
      </main>

      {updatePrompt.version && (
        <div
          className={`update-card${updatePrompt.busy ? " is-busy" : ""}`}
          role="status"
          aria-live="polite"
        >
          <span className="update-card-icon" aria-hidden>
            <RefreshIcon />
          </span>
          <div className="update-card-copy">
            <strong>{updatePrompt.busy ? "Installing update" : "Update available"}</strong>
            <span>
              {updatePrompt.busy
                ? "File Converter will restart when it's done."
                : `File Converter ${updatePrompt.version}`}
            </span>
          </div>
          {!updatePrompt.busy && (
            <button type="button" className="update-card-cta" onClick={updatePrompt.install}>
              Update
            </button>
          )}
          <button
            type="button"
            className="update-card-dismiss"
            disabled={updatePrompt.busy}
            onClick={updatePrompt.dismiss}
            aria-label="Dismiss"
          >
            <XIcon />
          </button>
          {updatePrompt.busy && <span className="update-card-bar" aria-hidden />}
        </div>
      )}
    </div>
  );
}
