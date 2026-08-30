import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";

export const SYNC_DIAGNOSTIC_FILE_NAME = "inkriver-sync-diagnostic.json";

/** Saves a redacted diagnostic through the native desktop or mobile picker. */
export async function saveSyncDiagnostic(
  contents: string,
  suggestedName = SYNC_DIAGNOSTIC_FILE_NAME,
): Promise<boolean> {
  const destination = await save({
    defaultPath: suggestedName,
    filters: [{ name: "Diagnostic InkRiver", extensions: ["json"] }],
  });
  if (destination === null) return false;
  await writeTextFile(destination, contents);
  return true;
}
