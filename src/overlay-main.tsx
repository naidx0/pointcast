import React from "react";
import ReactDOM from "react-dom/client";
import { Overlay } from "./windows/Overlay";
import { startPlatform, startTheme } from "./lib/theme";
import "./styles.css";

startPlatform();
startTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Overlay />
  </React.StrictMode>,
);
