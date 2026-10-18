import React from "react";
import { createRoot } from "react-dom/client";
import "./fonts";
import "streamdown/styles.css";
import "./index.css";
import { App } from "./App";
import { applyTextSize, textSizePref } from "./textSize";
import { applyTheme, resolveTheme, systemPrefersDark, themePref } from "./theme";

applyTheme(resolveTheme(themePref(), systemPrefersDark()));
applyTextSize(textSizePref());

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
