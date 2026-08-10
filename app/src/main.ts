import { openUrl } from "@tauri-apps/plugin-opener";
import { InkRiverApp } from "./app";
import { tauriApi } from "./api";
import "./styles.css";

const root = document.querySelector<HTMLElement>("#app");
if (!root) throw new Error("InkRiver root element is missing");

void new InkRiverApp(root, tauriApi, openUrl, (message) => window.confirm(message)).init();
