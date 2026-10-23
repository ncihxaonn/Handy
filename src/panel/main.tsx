import React from "react";
import ReactDOM from "react-dom/client";
import MenubarPanel from "./MenubarPanel";
import "@/i18n";
import "./MenubarPanel.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <MenubarPanel />
  </React.StrictMode>,
);
