import React from "react";
import ReactDOM from "react-dom/client";
import { Result } from "./windows/Result";
import { startPlatform, startTheme } from "./lib/theme";
import "./styles.css";

startPlatform();
startTheme();
document.body.classList.add("surface-app");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Result />
  </React.StrictMode>,
);
