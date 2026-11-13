import React from "react";
import ReactDOM from "react-dom/client";
import { Result } from "./windows/Result";
import { startPlatform, startTheme } from "./lib/theme";
import { startWindowDrag } from "./lib/api";
import "./styles.css";

startPlatform({ surface: true });
startTheme();
startWindowDrag();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Result />
  </React.StrictMode>,
);
