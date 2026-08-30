import { onBackButtonPress } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Format,
  checkPermissions,
  requestPermissions,
  scan,
} from "@tauri-apps/plugin-barcode-scanner";
import { openUrl } from "@tauri-apps/plugin-opener";
import { InkRiverApp } from "./app";
import { tauriApi } from "./api";
import { saveSyncDiagnostic } from "./diagnostic_export";
import "./styles.css";

const root = document.querySelector<HTMLElement>("#app");
if (!root) throw new Error("InkRiver root element is missing");

const isAndroid = /Android/i.test(navigator.userAgent);
const scanPairingCode = isAndroid
  ? async (): Promise<string> => {
      let permission = await checkPermissions();
      if (permission !== "granted") permission = await requestPermissions();
      if (permission !== "granted") {
        throw new Error("L’accès à la caméra est nécessaire pour scanner le QR code.");
      }
      const result = await scan({ cameraDirection: "back", formats: [Format.QRCode] });
      return result.content;
    }
  : null;

const app = new InkRiverApp(
  root,
  tauriApi,
  openUrl,
  (message) => window.confirm(message),
  scanPairingCode,
  saveSyncDiagnostic,
);
void app.init();

if (isAndroid) {
  void onBackButtonPress(({ canGoBack }) => {
    if (app.handleBackNavigation()) return;
    if (canGoBack) {
      window.history.back();
      return;
    }
    void getCurrentWindow().close();
  });
}
