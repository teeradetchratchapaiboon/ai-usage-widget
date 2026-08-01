/**
 * Zustand State Store - Central state management for the AI Usage Widget.
 *
 * Manages UI state (view, locale, loading, errors) and data fetched from
 * the Rust backend via the IPC bridge.
 */

import { create } from "zustand";
import {
  getCurrentUsage,
  getUsageHistory,
  getProviderStatus,
  triggerCollection,
  updateSettings as ipcUpdateSettings,
} from "../lib/ipc";
import type {
  UsageSummary,
  UsageRecord,
  ProviderStatus,
  AppSettings,
} from "../lib/ipc";

export interface AppState {
  // Data
  usage: UsageSummary | null;
  history: UsageRecord[];
  providers: ProviderStatus[];

  // UI state
  view: "compact" | "dashboard";
  locale: "th" | "en";
  /**
   * Per-request loading flags.
   *
   * One shared boolean did not survive concurrent requests: the widget fires
   * usage and provider-status together every 10 s, and whichever returned
   * first cleared the flag for both, so a skeleton could vanish while its own
   * request was still in flight.
   */
  usageLoading: boolean;
  providerStatusLoading: boolean;
  historyLoading: boolean;
  collectionLoading: boolean;
  settingsLoading: boolean;
  error: string | null;

  // Actions
  fetchUsage: () => Promise<void>;
  fetchHistory: (
    start: string,
    end: string,
    granularity: string,
  ) => Promise<void>;
  fetchProviderStatus: () => Promise<void>;
  triggerCollection: () => Promise<void>;
  updateSettings: (settings: Partial<AppSettings>) => Promise<void>;
  setView: (view: "compact" | "dashboard") => void;
  setLocale: (locale: "th" | "en") => void;
  clearError: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  // Initial data state
  usage: null,
  history: [],
  providers: [],

  // Initial UI state
  view: "compact",
  locale: "th",
  usageLoading: false,
  providerStatusLoading: false,
  historyLoading: false,
  collectionLoading: false,
  settingsLoading: false,
  error: null,

  // Actions
  fetchUsage: async () => {
    set({ usageLoading: true, error: null });
    try {
      const usage = await getCurrentUsage();
      set({ usage, usageLoading: false });
    } catch (e) {
      set({ error: String(e), usageLoading: false });
    }
  },

  fetchHistory: async (start: string, end: string, granularity: string) => {
    set({ historyLoading: true, error: null });
    try {
      const history = await getUsageHistory(start, end, granularity);
      set({ history, historyLoading: false });
    } catch (e) {
      set({ error: String(e), historyLoading: false });
    }
  },

  fetchProviderStatus: async () => {
    set({ providerStatusLoading: true, error: null });
    try {
      const providers = await getProviderStatus();
      set({ providers, providerStatusLoading: false });
    } catch (e) {
      set({ error: String(e), providerStatusLoading: false });
    }
  },

  triggerCollection: async () => {
    set({ collectionLoading: true, error: null });

    // Collection rewrites both the stored usage and the quota snapshot, so
    // both have to be re-read. Refreshing only usage left the dashboard's
    // quota cards showing pre-collection values after Collect Now.
    //
    // Each refresh is settled independently so one failure still delivers the
    // other's data, and both failures are reported rather than the last one.
    try {
      await triggerCollection();
    } catch (e) {
      set({ collectionLoading: false, error: String(e) });
      return;
    }

    const [usageResult, providersResult] = await Promise.allSettled([
      getCurrentUsage(),
      getProviderStatus(),
    ]);

    const errors: string[] = [];
    if (usageResult.status === "fulfilled") {
      set({ usage: usageResult.value });
    } else {
      errors.push(String(usageResult.reason));
    }
    if (providersResult.status === "fulfilled") {
      set({ providers: providersResult.value });
    } else {
      errors.push(String(providersResult.reason));
    }

    set({
      collectionLoading: false,
      error: errors.length > 0 ? errors.join("; ") : null,
    });
  },

  updateSettings: async (settings: Partial<AppSettings>) => {
    set({ settingsLoading: true, error: null });
    try {
      await ipcUpdateSettings(settings);
      set({ settingsLoading: false });
    } catch (e) {
      set({ error: String(e), settingsLoading: false });
    }
  },

  setView: (view) => set({ view }),

  setLocale: (locale) => set({ locale }),

  clearError: () => set({ error: null }),
}));
