import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { FormatGroup, QueuedFile } from "./types";
import { FileEmptyIcon } from "./icons";
import { FileListItem } from "./FileListItem";

const FORMAT_GROUP_CACHE: Partial<Record<string, FormatGroup[]>> = {};

interface FileListProps {
  files: QueuedFile[];
  onRemove: (id: string) => void;
  onFormatChange: (id: string, format: string) => void;
  onHlsRangeChange: (id: string, start: string, end: string) => void;
  onHlsDraftChange?: (id: string, start: string, end: string) => void;
  onYoutubeQualityChange: (id: string, quality: string) => void;
  onRename: (id: string, name: string) => void;
  onRevealInExplorer: (path: string) => void;
  onOpenFile: (path: string) => void;
  disabled?: boolean;
}

export function FileList({
  files,
  onRemove,
  onFormatChange,
  onHlsRangeChange,
  onHlsDraftChange,
  onYoutubeQualityChange,
  onRename,
  onRevealInExplorer,
  onOpenFile,
  disabled,
}: FileListProps) {
  const [groupsByCategory, setGroupsByCategory] = useState<
    Record<string, FormatGroup[]>
  >({});

  useEffect(() => {
    if (files.length === 0) return;

    const categories = ["video", "audio", "image"] as const;
    categories.forEach(async (cat) => {
      if (FORMAT_GROUP_CACHE[cat]) {
        setGroupsByCategory((prev) => ({
          ...prev,
          [cat]: FORMAT_GROUP_CACHE[cat]!,
        }));
        return;
      }
      const groups = await invoke<FormatGroup[]>("get_output_format_groups", {
        category: cat,
      });
      FORMAT_GROUP_CACHE[cat] = groups;
      setGroupsByCategory((prev) => ({ ...prev, [cat]: groups }));
    });
  }, [files.length]);

  if (files.length === 0) {
    return (
      <div className="empty-state">
        <div className="empty-icon">
          <FileEmptyIcon />
        </div>
        <p className="empty-hint">No files added yet</p>
      </div>
    );
  }

  return (
    <div className="file-list-scroll">
      <ul className="file-list">
        {files.map((file) => (
          <FileListItem
            key={file.id}
            file={file}
            formatGroups={groupsByCategory[file.category] ?? []}
            disabled={disabled}
            onRemove={onRemove}
            onFormatChange={onFormatChange}
            onHlsRangeChange={onHlsRangeChange}
            onHlsDraftChange={onHlsDraftChange}
            onYoutubeQualityChange={onYoutubeQualityChange}
            onRename={onRename}
            onRevealInExplorer={onRevealInExplorer}
            onOpenFile={onOpenFile}
          />
        ))}
      </ul>
    </div>
  );
}
