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
  isLoading: boolean;
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
  isLoading: false,
  error: null,

  // Actions
  fetchUsage: async () => {
    set({ isLoading: true, error: null });
    try {
      const usage = await getCurrentUsage();
      set({ usage, isLoading: false });
    } catch (e) {
      set({ error: String(e), isLoading: false });
    }
  },

  fetchHistory: async (start: string, end: string, granularity: string) => {
    set({ isLoading: true, error: null });
    try {
      const history = await getUsageHistory(start, end, granularity);
      set({ history, isLoading: false });
    } catch (e) {
      set({ error: String(e), isLoading: false });
    }
  },

  fetchProviderStatus: async () => {
    set({ isLoading: true, error: null });
    try {
      const providers = await getProviderStatus();
      set({ providers, isLoading: false });
    } catch (e) {
      set({ error: String(e), isLoading: false });
    }
  },

  triggerCollection: async () => {
    set({ isLoading: true, error: null });
    try {
      await triggerCollection();
      // Refresh usage data after collection
      const usage = await getCurrentUsage();
      set({ usage, isLoading: false });
    } catch (e) {
      set({ error: String(e), isLoading: false });
    }
  },

  updateSettings: async (settings: Partial<AppSettings>) => {
    set({ isLoading: true, error: null });
    try {
      await ipcUpdateSettings(settings);
      set({ isLoading: false });
    } catch (e) {
      set({ error: String(e), isLoading: false });
    }
  },

  setView: (view) => set({ view }),

  setLocale: (locale) => set({ locale }),

  clearError: () => set({ error: null }),
}));
