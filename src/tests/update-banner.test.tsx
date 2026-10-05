/**
 * UpdateBanner tests: detection, in-app install with progress, localized
 * failure with the release-page fallback, and dismissal.
 *
 * Mocks @tauri-apps/api/core (invoke + Channel) and the opener plugin.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import i18n from "../i18n";
import { UpdateBanner } from "../components/UpdateBanner";
import type { UpdateInfo, UpdateProgress } from "../lib/ipc";

// ─── Mocks ──────────────────────────────────────────────────────────────────────

const mockInvoke = vi.fn();
const mockOpenUrl = vi.fn();

// Hoisted with vi.mock, which runs before this module's own declarations
const { FakeChannel } = vi.hoisted(() => ({
  FakeChannel: class FakeChannel<T> {
    onmessage: (message: T) => void = () => {};
  },
}));
type FakeChannel<T> = InstanceType<typeof FakeChannel<T>>;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
  Channel: FakeChannel,
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

const info: UpdateInfo = {
  current_version: "0.2.0",
  latest_version: "0.3.0",
  download_url: "https://github.com/teeradetchratchapaiboon/ai-usage-widget/releases/tag/v0.3.0",
  release_notes: null,
};

type InstallImpl = (channel: FakeChannel<UpdateProgress>) => Promise<void>;

function setupMocks(update: UpdateInfo | null, installImpl?: InstallImpl) {
  mockInvoke.mockImplementation((cmd: string, args?: { onEvent?: FakeChannel<UpdateProgress> }) => {
    switch (cmd) {
      case "check_for_updates":
        return Promise.resolve(update);
      case "install_update":
        return installImpl ? installImpl(args!.onEvent!) : new Promise(() => {});
      default:
        return Promise.resolve(null);
    }
  });
}

function renderBanner() {
  return render(
    <I18nextProvider i18n={i18n}>
      <UpdateBanner />
    </I18nextProvider>,
  );
}

// ─── Tests ──────────────────────────────────────────────────────────────────────

describe("UpdateBanner", () => {
  beforeEach(async () => {
    await act(async () => {
      await i18n.changeLanguage("en");
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("renders when an update is available and nothing otherwise", async () => {
    setupMocks(info);
    const { unmount } = renderBanner();
    expect(await screen.findByText("Update available")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Update now" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Release page" })).toBeTruthy();
    unmount();

    setupMocks(null);
    const { container } = renderBanner();
    await act(async () => {});
    expect(container.innerHTML).toBe("");
  });

  it("installs through a channel and shows progress, then installing", async () => {
    let channel: FakeChannel<UpdateProgress> | null = null;
    setupMocks(info, (ch) => {
      channel = ch;
      return new Promise(() => {});
    });
    renderBanner();
    fireEvent.click(await screen.findByRole("button", { name: "Update now" }));

    await waitFor(() => expect(channel).not.toBeNull());
    const installCall = mockInvoke.mock.calls.find(([cmd]) => cmd === "install_update");
    expect(installCall?.[1]).toEqual({ onEvent: channel });
    expect(installCall?.[1].onEvent).toBeInstanceOf(FakeChannel);

    act(() => {
      channel!.onmessage({ event: "started", data: { content_length: 200 } });
      channel!.onmessage({ event: "progress", data: { chunk_length: 50 } });
    });
    const bar = screen.getByRole("progressbar", { name: "Update download progress" });
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(screen.getByText("Downloading… 25%")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Update now" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    // No dismissing mid-download
    expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();

    act(() => {
      channel!.onmessage({ event: "finished" });
    });
    expect(screen.getByText("Installing, the app will restart…")).toBeTruthy();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("shows a localized error and keeps the release page fallback", async () => {
    setupMocks(info, () => Promise.reject({ code: "UPDATE_NOT_AVAILABLE", message: "x" }));
    renderBanner();
    fireEvent.click(await screen.findByRole("button", { name: "Update now" }));

    expect(
      await screen.findByText(
        "Update failed: Automatic update is not available for this release. Use the release page instead.",
      ),
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Release page" }));
    expect(mockOpenUrl).toHaveBeenCalledWith(info.download_url);
    // Can try again after a failure
    expect((screen.getByRole("button", { name: "Update now" }) as HTMLButtonElement).disabled).toBe(
      false,
    );
  });

  it("hides when dismissed", async () => {
    setupMocks(info);
    renderBanner();
    fireEvent.click(await screen.findByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("Update available")).toBeNull();
  });
});
