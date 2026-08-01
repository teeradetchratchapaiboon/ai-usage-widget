/**
 * DashboardShell - Chrome for the dashboard window.
 *
 * Hosts the update banner and the two tabs (usage charts / settings). The
 * initial tab is injected as `window.__DASHBOARD_TAB__` before the window
 * boots; later tray clicks arrive as a `dashboard-tab` event.
 */

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dashboard } from "./Dashboard";
import { Settings } from "./Settings";
import { UpdateBanner } from "./UpdateBanner";
import { onAppEvent } from "../lib/tauri";
import { showWidget } from "../lib/ipc";

type Tab = "usage" | "settings";

declare global {
  interface Window {
    /** Injected by the Rust side when it creates the dashboard window. */
    __DASHBOARD_TAB__?: string;
  }
}

function initialTab(): Tab {
  if (typeof window === "undefined") return "usage";
  const requested =
    window.__DASHBOARD_TAB__ ??
    new URLSearchParams(window.location.search).get("tab");
  return requested === "settings" ? "settings" : "usage";
}

export function DashboardShell() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>(initialTab);

  useEffect(
    () =>
      onAppEvent<string>("dashboard-tab", (requested) => {
        setTab(requested === "settings" ? "settings" : "usage");
      }),
    [],
  );

  return (
    <div className="min-h-screen bg-neutral-900 text-white/90 flex flex-col">
      <UpdateBanner />

      <nav className="flex items-end gap-1 px-4 pt-3 border-b border-white/10">
        <TabButton
          active={tab === "usage"}
          onClick={() => setTab("usage")}
          label={t("dashboard.title")}
        />
        <TabButton
          active={tab === "settings"}
          onClick={() => setTab("settings")}
          label={t("settings.title")}
        />
        {/* The widget skips the taskbar, so this is the way back to it */}
        <button
          type="button"
          onClick={() => void showWidget()}
          className="ml-auto mb-1 px-3 py-1.5 text-xs rounded-md bg-white/10 text-white/70 hover:bg-white/20 hover:text-white transition-colors"
        >
          ← {t("tray.showWidget")}
        </button>
      </nav>

      <main className="flex-1 overflow-auto p-4">
        {tab === "usage" ? <Dashboard /> : <Settings />}
      </main>
    </div>
  );
}

interface TabButtonProps {
  active: boolean;
  label: string;
  onClick: () => void;
}

function TabButton({ active, label, onClick }: TabButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`px-4 py-2 text-sm rounded-t-md transition-colors ${
        active
          ? "bg-white/10 text-white border-b-2 border-blue-400"
          : "text-white/60 hover:text-white/90 hover:bg-white/5"
      }`}
    >
      {label}
    </button>
  );
}
