/**
 * Forwards uncaught webview errors to the Rust log.
 *
 * Release builds ship without devtools, so an exception during render would
 * otherwise leave nothing behind but a blank window.
 */

import { invoke } from "@tauri-apps/api/core";
import { currentWindowLabel, isTauri } from "./tauri";

function report(message: string): void {
  if (!isTauri()) return;
  invoke("log_frontend_error", {
    window: currentWindowLabel(),
    message,
  }).catch(() => {
    /* logging must never throw */
  });
}

export function installErrorReporting(): void {
  if (typeof window === "undefined") return;

  window.addEventListener("error", (event) => {
    const detail = event.error instanceof Error
      ? `${event.error.message}\n${event.error.stack ?? ""}`
      : String(event.message);
    report(`uncaught error: ${detail}`);
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    const detail = reason instanceof Error
      ? `${reason.message}\n${reason.stack ?? ""}`
      : String(reason);
    report(`unhandled rejection: ${detail}`);
  });
}
