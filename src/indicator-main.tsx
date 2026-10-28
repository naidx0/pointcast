import React from "react";
import ReactDOM from "react-dom/client";
import { Indicator } from "./windows/Indicator";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Indicator />
  </React.StrictMode>,
);
