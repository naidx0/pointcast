import React from "react";
import ReactDOM from "react-dom/client";
import { Settings } from "./windows/Settings";
import { startPlatform, startTheme } from "./lib/theme";
import { startWindowDrag } from "./lib/api";
import "./styles.css";

startPlatform({ surface: true });
startTheme();
startWindowDrag();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Settings />
  </React.StrictMode>,
);
