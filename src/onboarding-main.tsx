import React from "react";
import ReactDOM from "react-dom/client";
import { Onboarding } from "./windows/Onboarding";
import { startPlatform, startTheme } from "./lib/theme";
import "./styles.css";

startPlatform({ surface: true });
startTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Onboarding />
  </React.StrictMode>,
);
