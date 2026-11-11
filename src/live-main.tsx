import React from "react";
import ReactDOM from "react-dom/client";
import { Live } from "./windows/Live";
import { preview, startLivePreview } from "./lib/api";
import { startPlatform, startTheme } from "./lib/theme";
import "./styles.css";
import "./styles/live.css";

startPlatform();
startTheme();

if (preview) {
  // live.html?preview=1&state=listening|finishing|thinking|answering|shown|error&skin=mac|win&theme=light|dark
  const html = document.documentElement;
  const skin = new URLSearchParams(location.search).get("skin");
  if (skin === "mac" || skin === "win") {
    html.classList.remove("mac", "win", "linux");
    html.classList.add(skin);
  }
  html.classList.add("live-preview");
  startLivePreview();
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Live />
  </React.StrictMode>,
);
