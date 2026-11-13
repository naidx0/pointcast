import React from "react";
import ReactDOM from "react-dom/client";
import { Ring } from "./windows/Ring";
import { startPlatform, startTheme } from "./lib/theme";
import "./styles.css";
import "./styles/ring.css";

startPlatform();
startTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Ring />
  </React.StrictMode>,
);
