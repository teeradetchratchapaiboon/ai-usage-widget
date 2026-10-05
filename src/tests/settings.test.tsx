/**
 * Settings panel tests: sliders commit once on release, labels show the
 * saved thresholds, and every control has an accessible name.
 *
 * Mocks @tauri-apps/api/core to avoid needing a real Tauri runtime.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import i18n from "../i18n";
import en from "../i18n/en.json";
import th from "../i18n/th.json";
import { useAppStore } from "../store";
import { Settings } from "../components/Settings";
import type { AppSettings } from "../lib/ipc";

// ─── Mock @tauri-apps/api/core ──────────────────────────────────────────────────

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// ─── Fixture ────────────────────────────────────────────────────────────────────

const fixtureSettings: AppSettings = {
  collection_interval_secs: 60,
  locale: "en",
  notification_warning_pct: 60,
  notification_critical_pct: 85,
  autostart: false,
  always_on_top: false,
  click_through: false,
  data_dir: "C:\\Users\\test\\AppData\\Roaming\\ai-usage-widget",
};

function setupMocks(updateImpl: () => Promise<void> = () => Promise.resolve()) {
  mockInvoke.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "get_settings":
        return Promise.resolve({ ...fixtureSettings });
      case "update_settings":
        return updateImpl();
      default:
        return Promise.resolve(null);
    }
  });
}

function updateCalls() {
  return mockInvoke.mock.calls.filter(([cmd]) => cmd === "update_settings");
}

async function renderSettings() {
  render(
    <I18nextProvider i18n={i18n}>
      <Settings />
    </I18nextProvider>,
  );
  return screen.findByRole("slider", { name: "Collection Interval" });
}

// ─── Tests ──────────────────────────────────────────────────────────────────────

describe("Settings", () => {
  beforeEach(async () => {
    await act(async () => {
      await i18n.changeLanguage("en");
    });
    useAppStore.setState({ locale: "en" });
    setupMocks();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("commits the interval slider once on pointer release, not per step", async () => {
    const slider = await renderSettings();

    fireEvent.change(slider, { target: { value: "120" } });
    fireEvent.change(slider, { target: { value: "130" } });

    expect(updateCalls()).toHaveLength(0);
    expect(screen.getByText("130s")).toBeTruthy();

    fireEvent.pointerUp(slider);

    await waitFor(() => expect(updateCalls()).toHaveLength(1));
    expect(updateCalls()[0]).toEqual([
      "update_settings",
      { settings: { collection_interval_secs: 130 } },
    ]);

    // A blur after the release must not save the same value again
    fireEvent.blur(slider);
    await act(async () => {});
    expect(updateCalls()).toHaveLength(1);
  });

  it("commits the warning slider once on key release", async () => {
    await renderSettings();
    const warning = screen.getByRole("slider", { name: /Warning/ });

    fireEvent.change(warning, { target: { value: "70" } });
    expect(updateCalls()).toHaveLength(0);

    fireEvent.keyUp(warning, { key: "ArrowRight" });

    await waitFor(() => expect(updateCalls()).toHaveLength(1));
    expect(updateCalls()[0]).toEqual([
      "update_settings",
      { settings: { notification_warning_pct: 70 } },
    ]);
  });

  it("reverts the slider and shows the error when the backend rejects", async () => {
    setupMocks(() =>
      Promise.reject("Settings validation failed: warning threshold must be below critical"),
    );
    await renderSettings();
    const warning = screen.getByRole("slider", { name: /Warning/ });

    fireEvent.change(warning, { target: { value: "95" } });
    expect(screen.getByText("95%")).toBeTruthy();

    fireEvent.pointerUp(warning);

    expect(
      await screen.findByText(/warning threshold must be below critical/),
    ).toBeTruthy();
    await waitFor(() => expect(screen.getByText("60%")).toBeTruthy());
    expect(screen.queryByText("95%")).toBeNull();
    expect((warning as HTMLInputElement).value).toBe("60");
  });

  it("shows a backend error code as localized text", async () => {
    setupMocks(() =>
      Promise.reject({
        code: "THRESHOLD_ORDER",
        message: "Settings validation failed: invalid notification threshold: ...",
        params: { warning: "95", critical: "90" },
      }),
    );
    await renderSettings();
    const warning = screen.getByRole("slider", { name: /Warning/ });

    fireEvent.change(warning, { target: { value: "95" } });
    fireEvent.pointerUp(warning);

    const enText = "The warning threshold (95%) must be below the critical threshold (90%)";
    // Generous timeout: the default 1s was flaky on slower CI runners
    expect(await screen.findByText(enText, {}, { timeout: 5000 })).toBeTruthy();

    // Stored raw, so switching language re-translates the shown error
    await act(async () => {
      await i18n.changeLanguage("th");
    });
    expect(screen.getByText("เกณฑ์เตือน (95%) ต้องต่ำกว่าเกณฑ์วิกฤต (90%)")).toBeTruthy();
  });

  it("shows the saved threshold values instead of fixed defaults", async () => {
    await renderSettings();

    expect(screen.getByText("60%")).toBeTruthy();
    expect(screen.getByText("85%")).toBeTruthy();
    expect(screen.queryByText(/75%/)).toBeNull();
    expect(screen.queryByText(/90%/)).toBeNull();
    expect(screen.getByRole("slider", { name: /Warning/ })).toBeTruthy();
    expect(screen.getByRole("slider", { name: /Critical/ })).toBeTruthy();
  });

  it("gives every control an accessible name", async () => {
    await renderSettings();

    expect(screen.getByRole("switch", { name: "Autostart" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Always on Top" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Click-through" })).toBeTruthy();
    expect(screen.getByRole("slider", { name: "Collection Interval" })).toBeTruthy();

    const backup = screen.getByLabelText("Backup");
    expect(backup.tagName).toBe("INPUT");
    expect(backup.getAttribute("placeholder")).toBe(en.settings.backupPathPlaceholder);
  });

  it("marks the active language button as pressed", async () => {
    await renderSettings();

    expect(screen.getByRole("button", { name: "English" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
    expect(screen.getByRole("button", { name: "ไทย" }).getAttribute("aria-pressed")).toBe(
      "false",
    );
  });

  it("keeps the English and Thai settings keys in sync", () => {
    expect(Object.keys(en.settings).sort()).toEqual(Object.keys(th.settings).sort());
  });
});
