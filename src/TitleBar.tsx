import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogoIcon } from "./icons";

function MinimizeIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  );
}

function MaximizeIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <rect x="5" y="5" width="14" height="14" rx="1.5" />
    </svg>
  );
}

function RestoreIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <path d="M8 8h10v10H8z" />
      <path d="M6 6h10v2H8v8H6z" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <line x1="6" y1="6" x2="18" y2="18" />
      <line x1="18" y1="6" x2="6" y2="18" />
    </svg>
  );
}

export function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const dragRef = useRef<HTMLDivElement>(null);
  const appWindow = getCurrentWindow();

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    appWindow.isMaximized().then((m) => {
      if (!disposed) setMaximized(m);
    });

    appWindow
      .onResized(async () => {
        if (!disposed) setMaximized(await appWindow.isMaximized());
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const el = dragRef.current;
    if (!el) return;

    function onMouseDown(e: MouseEvent) {
      if (e.button !== 0) return;
      if (e.detail === 2) {
        appWindow.toggleMaximize();
      } else {
        appWindow.startDragging();
      }
    }

    el.addEventListener("mousedown", onMouseDown);
    return () => el.removeEventListener("mousedown", onMouseDown);
  }, []);

  return (
    <header className="titlebar">
      <div ref={dragRef} className="titlebar-drag">
        <div className="titlebar-brand">
          <div className="titlebar-logo">
            <LogoIcon />
          </div>
          <div className="titlebar-text">
            <span className="titlebar-title">File Converter</span>
            <span className="titlebar-subtitle">
              Fast, quality-preserving video, audio, and image conversion
            </span>
          </div>
        </div>
      </div>
      <div className="titlebar-controls">
        <button
          type="button"
          className="titlebar-btn"
          aria-label="Minimize"
          onClick={() => appWindow.minimize()}
        >
          <MinimizeIcon />
        </button>
        <button
          type="button"
          className="titlebar-btn"
          aria-label={maximized ? "Restore" : "Maximize"}
          onClick={() => appWindow.toggleMaximize()}
        >
          {maximized ? <RestoreIcon /> : <MaximizeIcon />}
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-btn-close"
          aria-label="Close"
          onClick={() => appWindow.close()}
        >
          <CloseIcon />
        </button>
      </div>
    </header>
  );
}
