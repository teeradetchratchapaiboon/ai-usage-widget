import React from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import "./i18n";
import { installErrorReporting } from "./lib/errors";
import App from "./App";

installErrorReporting();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
