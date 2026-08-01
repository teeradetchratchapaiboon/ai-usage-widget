/**
 * Tests for the countdown formatter.
 *
 * The reset line is the only place the widget makes a claim about the future,
 * so its edge cases matter more than its happy path: a stale timestamp that
 * still renders, or a full week that reads as six days, are both lies told
 * confidently in 9-pixel text.
 */

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import i18n from "../i18n";
import { formatCountdown } from "../lib/format";

/** An ISO timestamp `seconds` away from the frozen clock. */
function inSeconds(seconds: number): string {
  return new Date(Date.now() + seconds * 1000).toISOString();
}

const HOUR = 3600;
const DAY = 86400;

describe("formatCountdown", () => {
  beforeEach(async () => {
    // Freeze time: every expectation below is exact, not "about"
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-01T12:00:00Z"));
    await i18n.changeLanguage("en");
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("drops a reset that has already passed instead of promising one", async () => {
    // Codex publishes its five-hour reset verbatim and never retracts it, so
    // an app left idle overnight holds a timestamp hours in the past. Calling
    // that "any moment" would be a standing false claim; saying nothing is the
    // honest reading of a number too old to mean anything.
    expect(formatCountdown(inSeconds(-1))).toBeNull();
    expect(formatCountdown(inSeconds(-12 * HOUR))).toBeNull();
    expect(formatCountdown(inSeconds(0))).toBeNull();
  });

  it("returns null rather than a placeholder for unusable input", () => {
    expect(formatCountdown(null)).toBeNull();
    expect(formatCountdown("")).toBeNull();
    expect(formatCountdown("not a date")).toBeNull();
  });

  it("reports a whole window as whole, not one unit short", () => {
    // Truncating turns a freshly issued weekly quota into "6 days 23 hrs",
    // which reads as a bug rather than as a full week.
    expect(formatCountdown(inSeconds(7 * DAY))).toBe("in 7 days");
    expect(formatCountdown(inSeconds(5 * HOUR))).toBe("in 5 hrs");
  });

  it("agrees with the number it prints", () => {
    expect(formatCountdown(inSeconds(DAY + 2 * HOUR))).toBe("in 1 day 2 hrs");
    expect(formatCountdown(inSeconds(HOUR + 60))).toBe("in 1 hr 1 min");
    expect(formatCountdown(inSeconds(2 * DAY + 3 * HOUR))).toBe("in 2 days 3 hrs");
  });

  it("carries a rounded unit up instead of printing 24 hrs or 60 min", () => {
    // 1 day 23h 40m rounds the hours to 24, which has to become a day
    expect(formatCountdown(inSeconds(DAY + 23 * HOUR + 40 * 60))).toBe("in 2 days");
    // 2h 59m 40s rounds the minutes to 60, which has to become an hour
    expect(formatCountdown(inSeconds(2 * HOUR + 59 * 60 + 40))).toBe("in 3 hrs");
  });

  it("never counts down to zero while time is still left", () => {
    // Under a minute is still a wait; "0 min" reads as a stopped clock
    expect(formatCountdown(inSeconds(30))).toBe("in 1 min");
    expect(formatCountdown(inSeconds(1))).toBe("in 1 min");
  });

  it("stops at two units, the most precision the decision needs", () => {
    expect(formatCountdown(inSeconds(3 * DAY + 4 * HOUR + 20 * 60))).toBe("in 3 days 4 hrs");
  });

  it("rounds the wait up rather than down when it lands mid-unit", () => {
    // Rounding the smallest shown unit can only move the stated wait by half
    // a unit. Erring long is the safe direction: a user who thinks they wait
    // longer than they do loses nothing, the reverse plans around a quota
    // that has not come back yet.
    expect(formatCountdown(inSeconds(3 * DAY + 4 * HOUR + 30 * 60))).toBe("in 3 days 5 hrs");
  });

  it("marks an approximate wait on the number, not the whole phrase", () => {
    // Claude's resets are reconstructed from its history; the uncertainty is
    // in the quantity, so "in ~4 hrs" is where the marker belongs.
    expect(formatCountdown(inSeconds(4 * HOUR), { approximate: true })).toBe("in ~4 hrs");
    expect(formatCountdown(inSeconds(4 * HOUR), { approximate: false })).toBe("in 4 hrs");
    expect(formatCountdown(inSeconds(4 * HOUR))).toBe("in 4 hrs");
  });

  it("formats Thai without leaking untranslated keys", async () => {
    await i18n.changeLanguage("th");

    expect(formatCountdown(inSeconds(DAY + 2 * HOUR))).toBe("อีก 1 วัน 2 ชม.");
    expect(formatCountdown(inSeconds(4 * HOUR), { approximate: true })).toBe("อีก ~4 ชม.");
    expect(formatCountdown(inSeconds(90))).toBe("อีก 2 น.");

    await i18n.changeLanguage("en");
  });
});
