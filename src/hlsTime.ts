export function formatHlsTime(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs)) return "";
  const total = Math.max(0, Math.floor(secs));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  if (m > 0) {
    return `${m}:${String(s).padStart(2, "0")}`;
  }
  return String(s);
}

export function parseHlsTimeInput(input: string): string {
  return input.trim();
}
