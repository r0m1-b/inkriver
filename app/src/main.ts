import { onBackButtonPress } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { InkRiverApp } from "./app";
import { tauriApi } from "./api";
import "./styles.css";

const root = document.querySelector<HTMLElement>("#app");
if (!root) throw new Error("InkRiver root element is missing");

const app = new InkRiverApp(root, tauriApi, openUrl, (message) => window.confirm(message));
void app.init();

if (/Android/i.test(navigator.userAgent)) {
  void onBackButtonPress(({ canGoBack }) => {
    if (app.handleBackNavigation()) return;
    if (canGoBack) {
      window.history.back();
      return;
    }
    void getCurrentWindow().close();
  });
}
