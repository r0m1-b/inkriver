import { beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({
  save: vi.fn(),
  writeTextFile: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: native.save }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: native.writeTextFile }));

import { saveSyncDiagnostic } from "./diagnostic_export";

describe("saveSyncDiagnostic", () => {
  beforeEach(() => {
    native.save.mockReset();
    native.writeTextFile.mockReset();
    native.writeTextFile.mockResolvedValue(undefined);
  });

  it("writes the diagnostic to a desktop path selected by the user", async () => {
    native.save.mockResolvedValue("/tmp/inkriver-diagnostic.json");

    await expect(saveSyncDiagnostic('{"format":"inkriver-sync-diagnostic"}'))
      .resolves.toBe(true);

    expect(native.save).toHaveBeenCalledWith({
      defaultPath: "inkriver-sync-diagnostic.json",
      filters: [{ name: "Diagnostic InkRiver", extensions: ["json"] }],
    });
    expect(native.writeTextFile).toHaveBeenCalledWith(
      "/tmp/inkriver-diagnostic.json",
      '{"format":"inkriver-sync-diagnostic"}',
    );
  });

  it("passes an Android content URI unchanged to the filesystem plugin", async () => {
    const contentUri = "content://com.android.providers.downloads.documents/document/42";
    native.save.mockResolvedValue(contentUri);

    await expect(saveSyncDiagnostic("{}", "support.json")).resolves.toBe(true);

    expect(native.writeTextFile).toHaveBeenCalledWith(contentUri, "{}");
  });

  it("does not write anything when the native picker is cancelled", async () => {
    native.save.mockResolvedValue(null);

    await expect(saveSyncDiagnostic("{}")).resolves.toBe(false);

    expect(native.writeTextFile).not.toHaveBeenCalled();
  });
});
