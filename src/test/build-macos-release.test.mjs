import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const SCRIPT = "scripts/build-macos-release.sh";

// `--check` prints the plan the variables select and exits before building,
// importing a certificate or running codesign.
function plan(env = {}) {
  const result = spawnSync("bash", [SCRIPT, "--check"], {
    encoding: "utf8",
    env: {
      PATH: process.env.PATH,
      HOME: process.env.HOME,
      SIGNING_ENV_FILE: "/nonexistent/.env.signing",
      ...env,
    },
  });
  const lines = Object.fromEntries(
    result.stdout
      .trim()
      .split("\n")
      .filter((line) => line.includes("="))
      .map((line) => line.split("=")),
  );
  return { status: result.status, stderr: result.stderr, plan: lines };
}

const notarization = {
  APPLE_API_ISSUER: "issuer",
  APPLE_API_KEY: "KEYID",
  APPLE_API_KEY_P8_BASE64: "cDg=",
};

describe("macOS release build plan", () => {
  it("prepares the Local resource before any Computer use Cargo build", () => {
    const script = readFileSync(SCRIPT, "utf8");
    const local = script.indexOf("pnpm local-transcriber:prepare -- --release");
    const computerUse = script.indexOf("pnpm computer-use:prepare");
    expect(local).toBeGreaterThanOrEqual(0);
    expect(local).toBeLessThan(computerUse);
  });

  it("signs ad hoc and skips notarization without Apple variables", () => {
    const { status, plan: result } = plan();
    expect(status).toBe(0);
    expect(result).toMatchObject({ signing: "adhoc", notarize: "no", updater: "off" });
  });

  it("signs with the identity when one is set", () => {
    const { status, plan: result } = plan({
      APPLE_SIGNING_IDENTITY: "Developer ID Application: X",
    });
    expect(status).toBe(0);
    expect(result).toMatchObject({ signing: "identity", notarize: "no" });
  });

  it("notarizes only with the identity and the App Store Connect key", () => {
    const { status, plan: result } = plan({
      APPLE_SIGNING_IDENTITY: "Developer ID Application: X",
      ...notarization,
    });
    expect(status).toBe(0);
    expect(result).toMatchObject({ signing: "identity", notarize: "yes" });
  });

  it("refuses notarization settings without an identity", () => {
    const { status, stderr } = plan(notarization);
    expect(status).not.toBe(0);
    expect(stderr).toContain("APPLE_SIGNING_IDENTITY");
  });

  it("refuses incomplete notarization settings", () => {
    const { status, stderr } = plan({
      APPLE_SIGNING_IDENTITY: "Developer ID Application: X",
      APPLE_API_KEY: "KEYID",
    });
    expect(status).not.toBe(0);
    expect(stderr).toContain("APPLE_API_ISSUER");
  });

  it("passes no Apple variable to Tauri when CI sends absent secrets as empty", () => {
    const empty = Object.fromEntries(
      [
        "APPLE_SIGNING_IDENTITY",
        "APPLE_CERTIFICATE",
        "APPLE_CERTIFICATE_PASSWORD",
        "APPLE_API_ISSUER",
        "APPLE_API_KEY",
        "APPLE_API_KEY_P8_BASE64",
      ].map((name) => [name, ""]),
    );
    const { status, plan: result } = plan(empty);
    expect(status).toBe(0);
    expect(result).toMatchObject({ signing: "adhoc", notarize: "no", tauri_apple_env: "" });
  });

  it("keeps an ad hoc build from importing a certificate it was given", () => {
    const { plan: result } = plan({ APPLE_CERTIFICATE: "cDEy", APPLE_CERTIFICATE_PASSWORD: "x" });
    expect(result).toMatchObject({ signing: "adhoc", tauri_apple_env: "" });
  });

  it("creates signed updater artifacts when the updater key is present", () => {
    const { status, plan: result } = plan({ TAURI_SIGNING_PRIVATE_KEY: "key" });
    expect(status).toBe(0);
    expect(result).toMatchObject({ signing: "adhoc", updater: "on" });
  });

  it("replaces the signed-only DMG script", () => {
    const pkg = JSON.parse(readFileSync("package.json", "utf8"));
    expect(pkg.scripts["tauri:build:release-mac"]).toBe(`./${SCRIPT}`);
    expect(pkg.scripts["tauri:build:signed-dmg"]).toBeUndefined();
    expect(existsSync("scripts/build-signed-dmg.sh")).toBe(false);
    const script = readFileSync(SCRIPT, "utf8");
    expect(script).toContain("CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none");
    expect(script).toContain("src-tauri/tauri.release.conf.json");
    expect(script).toContain("clovy-signing.keychain-db");
    // An ad hoc release cannot carry the Developer ID the helper check expects.
    expect(script).toContain("computer_use_self_test_args+=(--adhoc)");
    const selfTest = readFileSync("scripts/computer-use-release-self-test.sh", "utf8");
    expect(selfTest).toContain("args+=(--require-release-profile)");
  });
});
