import React from "react";
import { createRoot } from "react-dom/client";
import "./fonts";
import "streamdown/styles.css";
import "./index.css";
import { App } from "./App";
import { applyTheme, resolveTheme, systemPrefersDark, themePref } from "./theme";

applyTheme(resolveTheme(themePref(), systemPrefersDark()));

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
