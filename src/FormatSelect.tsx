import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { FormatGroup } from "./types";
import { ChevronDownIcon } from "./icons";

interface FormatSelectProps {
  value: string;
  groups: FormatGroup[];
  disabled?: boolean;
  ariaLabel: string;
  onChange: (value: string) => void;
}

interface MenuPosition {
  top: number;
  left: number;
  minWidth: number;
  maxHeight: number;
  placement: "above" | "below";
}

export function FormatSelect({
  value,
  groups,
  disabled,
  ariaLabel,
  onChange,
}: FormatSelectProps) {
  const [open, setOpen] = useState(false);
  const [menuPos, setMenuPos] = useState<MenuPosition | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;

    const rect = trigger.getBoundingClientRect();
    const gap = 6;
    const padding = 8;
    const estimatedHeight = 220;
    const spaceBelow = window.innerHeight - rect.bottom - padding;
    const spaceAbove = rect.top - padding;
    const openAbove = spaceBelow < estimatedHeight && spaceAbove > spaceBelow;

    const maxHeight = Math.min(
      240,
      openAbove ? spaceAbove - gap : spaceBelow - gap,
    );

    setMenuPos({
      top: openAbove ? rect.top - gap : rect.bottom + gap,
      left: rect.left,
      minWidth: Math.max(rect.width, 148),
      maxHeight: Math.max(maxHeight, 120),
      placement: openAbove ? "above" : "below",
    });
  }, []);

  useLayoutEffect(() => {
    if (!open) {
      setMenuPos(null);
      return;
    }
    updatePosition();
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open) return;

    function handlePointerDown(e: PointerEvent) {
      const target = e.target as Node;
      if (
        wrapRef.current?.contains(target) ||
        menuRef.current?.contains(target)
      ) {
        return;
      }
      setOpen(false);
    }

    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }

    function handleReposition() {
      updatePosition();
    }

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKey);
    window.addEventListener("resize", handleReposition);
    window.addEventListener("scroll", handleReposition, true);

    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKey);
      window.removeEventListener("resize", handleReposition);
      window.removeEventListener("scroll", handleReposition, true);
    };
  }, [open, updatePosition]);

  function toggle() {
    if (!disabled) setOpen((o) => !o);
  }

  function select(fmt: string) {
    onChange(fmt);
    setOpen(false);
  }

  const menu =
    open &&
    menuPos &&
    createPortal(
      <div
        ref={menuRef}
        className={`format-select-menu format-select-menu-${menuPos.placement}`}
        role="listbox"
        aria-label={ariaLabel}
        style={{
          top: menuPos.top,
          left: menuPos.left,
          minWidth: menuPos.minWidth,
          maxHeight: menuPos.maxHeight,
        }}
      >
        <div className="format-select-menu-inner">
          {groups.map((group) => (
            <div key={group.label} className="format-select-group">
              <div className="format-select-group-label">{group.label}</div>
              {group.formats.map((fmt) => (
                <button
                  key={`${group.label}-${fmt}`}
                  type="button"
                  role="option"
                  aria-selected={fmt === value}
                  className={`format-select-option${fmt === value ? " selected" : ""}`}
                  onClick={() => select(fmt)}
                >
                  <span className="format-select-option-text">{fmt}</span>
                  {fmt === value && (
                    <span className="format-select-check" aria-hidden="true" />
                  )}
                </button>
              ))}
            </div>
          ))}
        </div>
      </div>,
      document.body,
    );

  return (
    <div className={`format-select${open ? " open" : ""}`} ref={wrapRef}>
      <button
        ref={triggerRef}
        type="button"
        className="format-select-trigger"
        disabled={disabled}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={toggle}
      >
        <span className="format-select-value">{value}</span>
        <ChevronDownIcon className="format-select-chevron" />
      </button>
      {menu}
    </div>
  );
}
