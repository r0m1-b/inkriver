import { openUrl } from "@tauri-apps/plugin-opener";
import { ReaderApp } from "./app";
import { tauriApi } from "./api";
import "./styles.css";

const root = document.querySelector<HTMLElement>("#app");
if (!root) throw new Error("Reader root element is missing");

void new ReaderApp(root, tauriApi, openUrl, (message) => window.confirm(message)).init();
