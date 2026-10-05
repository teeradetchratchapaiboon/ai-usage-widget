/**
 * Backend error-code formatting: codes from src-tauri/src/command_error.rs
 * map to localized text, with the English message as fallback.
 */

import { describe, it, expect, afterAll } from "vitest";
import i18n from "../i18n";
import en from "../i18n/en.json";
import th from "../i18n/th.json";
import { formatError, isCommandError } from "../lib/commandError";

function keysOf(obj: unknown, prefix = ""): string[] {
  if (typeof obj !== "object" || obj === null) return [prefix];
  return Object.entries(obj as Record<string, unknown>)
    .flatMap(([k, v]) => keysOf(v, prefix ? `${prefix}.${k}` : k))
    .sort();
}

const intervalError = {
  code: "INVALID_INTERVAL",
  message: "Settings validation failed: invalid interval: 5s (must be between 10s and 3600s)",
  params: { value: "5", min: "10", max: "3600" },
};

describe("formatError", () => {
  afterAll(async () => {
    await i18n.changeLanguage("th");
  });

  it("renders a known code in English with params interpolated", async () => {
    await i18n.changeLanguage("en");
    expect(formatError(intervalError, i18n.t)).toBe(
      "Collection interval must be between 10 and 3600 seconds (got 5)",
    );
  });

  it("renders a known code in Thai with params interpolated", async () => {
    await i18n.changeLanguage("th");
    expect(formatError(intervalError, i18n.t)).toBe(
      "ช่วงเวลาเก็บข้อมูลต้องอยู่ระหว่าง 10 ถึง 3600 วินาที (ได้รับ 5)",
    );
  });

  it("falls back to the message for an unknown code", async () => {
    await i18n.changeLanguage("en");
    expect(formatError({ code: "SOMETHING_NEW", message: "raw detail" }, i18n.t)).toBe(
      "raw detail",
    );
  });

  it("passes plain strings and Error objects through", () => {
    expect(formatError("plain failure", i18n.t)).toBe("plain failure");
    expect(formatError(new Error("boom"), i18n.t)).toBe("boom");
    expect(formatError(42, i18n.t)).toBe("42");
  });

  it("recognises only objects with string code and message", () => {
    expect(isCommandError(intervalError)).toBe(true);
    expect(isCommandError({ code: 1, message: "x" })).toBe(false);
    expect(isCommandError({ code: "X" })).toBe(false);
    expect(isCommandError("X")).toBe(false);
    expect(isCommandError(null)).toBe(false);
  });

  it("keeps the English and Thai error-code and update keys in sync", () => {
    expect(keysOf(en.errors.code)).toEqual(keysOf(th.errors.code));
    expect(keysOf(en.update)).toEqual(keysOf(th.update));
  });
});
