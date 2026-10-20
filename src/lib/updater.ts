import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateCheckResult =
  | { kind: "available"; version: string }
  | { kind: "none" }
  | { kind: "skipped" }
  | { kind: "error" };

let pending: Update | null = null;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Check for a newer release. Keeps the update handle for {@link installPendingUpdate}. */
export async function checkForUpdate(): Promise<UpdateCheckResult> {
  if (!isTauri()) return { kind: "skipped" };
  try {
    const update = await check({
      headers: { "User-Agent": "File Converter" },
    });
    if (!update) {
      pending = null;
      return { kind: "none" };
    }
    pending = update;
    return { kind: "available", version: update.version };
  } catch {
    pending = null;
    return { kind: "error" };
  }
}

/** Download/install the update found by {@link checkForUpdate}, then relaunch. */
export async function installPendingUpdate(): Promise<"updated" | "none" | "error"> {
  if (!isTauri()) return "none";
  try {
    if (!pending) {
      const result = await checkForUpdate();
      if (result.kind !== "available") return result.kind === "error" ? "error" : "none";
    }
    const update = pending;
    if (!update) return "none";
    await update.downloadAndInstall();
    pending = null;
    await relaunch();
    return "updated";
  } catch {
    return "error";
  }
}
