import React from "react";
import ReactDOM from "react-dom/client";
import { Indicator } from "./windows/Indicator";
import { startPlatform, startTheme } from "./lib/theme";
import "./styles.css";

startPlatform();
startTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Indicator />
  </React.StrictMode>,
);
