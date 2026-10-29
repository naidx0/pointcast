import React from "react";
import ReactDOM from "react-dom/client";
import { Settings } from "./windows/Settings";
import { startPlatform, startTheme } from "./lib/theme";
import "./styles.css";

startPlatform({ surface: true });
startTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Settings />
  </React.StrictMode>,
);
