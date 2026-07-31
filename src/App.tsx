/**
 * App - Picks the view for the window it is running in.
 *
 * The compact widget and the dashboard share one bundle: the widget window
 * ("main") renders CompactWidget, the dashboard window renders DashboardShell.
 * Tray menu events that apply to both windows are handled here.
 */

import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { CompactWidget } from "./components/CompactWidget";
import { DashboardShell } from "./components/DashboardShell";
import { useAppStore } from "./store";
import { currentWindowLabel, onAppEvent } from "./lib/tauri";
import "./App.css";

function App() {
  const { i18n } = useTranslation();
  const setLocale = useAppStore((s) => s.setLocale);
  const triggerCollection = useAppStore((s) => s.triggerCollection);
  const fetchUsage = useAppStore((s) => s.fetchUsage);
  const isDashboard = currentWindowLabel() === "dashboard";

  useEffect(() => {
    const disposers = [
      onAppEvent("toggle-locale", () => {
        const next = i18n.language === "th" ? "en" : "th";
        void i18n.changeLanguage(next);
        setLocale(next);
      }),
      onAppEvent("collect-now", async () => {
        await triggerCollection();
        await fetchUsage();
      }),
    ];

    return () => disposers.forEach((dispose) => dispose());
  }, [i18n, setLocale, triggerCollection, fetchUsage]);

  return isDashboard ? <DashboardShell /> : <CompactWidget />;
}

export default App;
