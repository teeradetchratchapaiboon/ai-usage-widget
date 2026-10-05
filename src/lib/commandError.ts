/**
 * Formatting of errors returned by Tauri commands.
 *
 * Converted commands (settings, backup/restore, updates) reject with a
 * structured `{ code, message, params }` from src-tauri/src/command_error.rs.
 * `code` maps to the `errors.code.<CODE>` i18n key with `params` as
 * interpolation values; `message` (English detail) is the fallback. Other
 * commands still reject with plain strings, which pass through unchanged.
 */

import type { TFunction } from "i18next";

export interface CommandError {
  code: string;
  message: string;
  params?: Record<string, string>;
}

export function isCommandError(e: unknown): e is CommandError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { code?: unknown }).code === "string" &&
    typeof (e as { message?: unknown }).message === "string"
  );
}

/** Human-readable text for any rejected command value, in the current locale. */
export function formatError(e: unknown, t: TFunction): string {
  if (isCommandError(e)) {
    return t(`errors.code.${e.code}`, { ...e.params, defaultValue: e.message });
  }
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
