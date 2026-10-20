import { UploadIcon } from "./icons";

interface DropZoneProps {
  onBrowse: () => void;
  disabled?: boolean;
  dragActive?: boolean;
  isProbing?: boolean;
}

export function DropZone({ onBrowse, disabled, dragActive, isProbing }: DropZoneProps) {
  const inactive = disabled || isProbing;
  const classes = [
    "drop-zone",
    inactive && "disabled",
    dragActive && "drag-active",
    isProbing && "probing",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      className={classes}
      onClick={() => !inactive && onBrowse()}
      role="button"
      tabIndex={inactive ? -1 : 0}
      aria-disabled={inactive}
      aria-busy={isProbing}
      onKeyDown={(e) => {
        if (inactive) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onBrowse();
        }
      }}
    >
      <div className="drop-icon-wrap">
        {isProbing ? <span className="spinner" /> : <UploadIcon />}
      </div>
      <p className="drop-title">
        {isProbing
          ? "Analyzing files…"
          : dragActive
            ? "Release to add files"
            : "Drop files here"}
      </p>
      <p className="drop-sub">or click to browse — video, audio, images, or paste a media / HLS link</p>
    </div>
  );
}
