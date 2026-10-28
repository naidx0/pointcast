import React from "react";
import ReactDOM from "react-dom/client";
import { Settings } from "./windows/Settings";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Settings />
  </React.StrictMode>,
);
