import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const tauriConfig = readJson("src-tauri/tauri.conf.json");
const releaseConfig = readJson("src-tauri/tauri.release.conf.json");
const platformConfigs = [
  readJson("src-tauri/tauri.macos.conf.json"),
  readJson("src-tauri/tauri.windows.conf.json"),
];

// The owner's updater key (minisign key id), generated for release-propria.
const OWNER_KEY_ID = "7C81D3CAB6D75836";
const LATEST_MANIFEST =
  "https://github.com/guilhermexp/os-clovy/releases/latest/download/latest.json";

describe("updater configuration", () => {
  it("trusts only the owner's public key and one stable manifest", () => {
    const { pubkey, endpoints } = tauriConfig.plugins.updater;
    const decoded = Buffer.from(pubkey, "base64").toString("utf8");
    expect(decoded).toContain(`minisign public key: ${OWNER_KEY_ID}`);
    expect(endpoints).toEqual([LATEST_MANIFEST]);
    for (const config of platformConfigs) {
      expect(config.plugins?.updater).toBeUndefined();
    }
  });

  it("creates updater artifacts only through the release overlay", () => {
    expect(tauriConfig.bundle.createUpdaterArtifacts).toBeUndefined();
    for (const config of platformConfigs) {
      expect(config.bundle?.createUpdaterArtifacts).toBeUndefined();
    }
    expect(releaseConfig).toEqual({ bundle: { createUpdaterArtifacts: true } });
  });

  it("registers the updater plugin in the native shell", () => {
    expect(readFileSync("src-tauri/Cargo.toml", "utf8")).toMatch(/^tauri-plugin-updater = "/m);
    expect(readFileSync("src-tauri/src/lib.rs", "utf8")).toContain(
      ".plugin(tauri_plugin_updater::Builder::new().build())",
    );
  });
});
