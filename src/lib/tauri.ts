/**
 * Thin wrappers around the Tauri window/event APIs.
 *
 * Everything here degrades to a no-op outside a Tauri webview so the
 * components stay renderable in jsdom tests.
 */

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** True when running inside a Tauri webview. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Label of the window this bundle is running in ("main", "dashboard", …). */
export function currentWindowLabel(): string {
  if (!isTauri()) {
    // Outside Tauri (browser preview) the view can be forced with ?view=
    if (typeof window !== "undefined") {
      const view = new URLSearchParams(window.location.search).get("view");
      if (view) return view;
    }
    return "main";
  }
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

/**
 * Subscribe to a backend event. Returns a disposer that is safe to call even
 * if the subscription never completed.
 */
export function onAppEvent<T>(
  event: string,
  handler: (payload: T) => void | Promise<void>,
): () => void {
  if (!isTauri()) return () => {};

  let cancelled = false;
  let unlisten: (() => void) | null = null;

  listen<T>(event, (e) => {
    void handler(e.payload);
  })
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch((err) => console.warn(`Failed to listen for '${event}':`, err));

  return () => {
    cancelled = true;
    unlisten?.();
  };
}

/** Toggle the current window between maximized and its previous size. */
export async function toggleMaximizeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await getCurrentWindow().toggleMaximize();
  } catch (err) {
    console.warn("Failed to toggle maximize:", err);
  }
}
