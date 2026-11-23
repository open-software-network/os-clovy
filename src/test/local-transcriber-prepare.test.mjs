import { createHash } from "node:crypto";
import * as fs from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import vm from "node:vm";
import { afterEach, expect, it } from "vitest";

const script = fs.readFileSync(path.resolve("scripts/prepare-local-transcriber.mjs"), "utf8");
const sandboxes = [];
afterEach(() => {
  for (const root of sandboxes.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

function sandbox({ release = false } = {}) {
  const root = fs.mkdtempSync(path.join(tmpdir(), "clovy-local-build-"));
  sandboxes.push(root);
  const source = path.join(root, "src-tauri/native/mac-local-transcriber");
  fs.mkdirSync(path.join(source, "Sources"), { recursive: true });
  const hash = createHash("sha256");
  for (const file of [
    "Package.swift",
    "Package.resolved",
    "Sources/main.swift",
    "FluidAudio-LICENSE",
  ]) {
    fs.writeFileSync(path.join(source, file), file);
    hash.update(file).update("\0").update(file).update("\0");
  }
  fs.writeFileSync(
    path.join(source, "pin.json"),
    JSON.stringify({ sourceSha256: hash.digest("hex") }),
  );
  fs.writeFileSync(
    path.join(root, "src-tauri/tauri.conf.json"),
    JSON.stringify({ identifier: "test.local" }),
  );
  const bin = path.join(root, "build");
  fs.mkdirSync(bin);
  const executable = path.join(
    root,
    ".tauri-helper/Clovy Local Transcriber.app/Contents/MacOS/clovy-local-transcriber",
  );
  let builds = 0;
  const signatures = [];
  const swiftEnvironments = [];
  function run() {
    // Run the real packaging logic, with filesystem fixtures and system tools isolated.
    vm.runInNewContext(
      script.replace(/^import[\s\S]*?;\n/gm, "").replaceAll("import.meta.url", "scriptUrl"),
      {
        ...fs,
        existsSync: (file) =>
          file === "/Applications/Xcode.app/Contents/Developer" || fs.existsSync(file),
        createHash,
        path,
        fileURLToPath,
        scriptUrl: pathToFileURL(path.join(root, "scripts/prepare-local-transcriber.mjs")).href,
        process: {
          platform: "darwin",
          env: release ? { APPLE_SIGNING_IDENTITY: "Developer ID fixture" } : {},
          argv: release ? ["--release"] : [],
        },
        DEV_SIGNING_IDENTITY_ENV: "CLOVY_DEV_SIGNING_IDENTITY",
        resolveDevSigningIdentity: () => ({ identity: "fixture-certificate" }),
        console: { error() {} },
        computerUseBundleIdentifier: () => "test.local.helper",
        signatureSatisfies: () => true,
        spawnSync(command, args, options) {
          if (command === "swift") {
            swiftEnvironments.push(options.env);
            if (args.includes("--show-bin-path")) return { status: 0, stdout: bin };
            builds += 1;
            fs.writeFileSync(path.join(bin, "clovy-local-transcriber"), "trusted-build");
          } else if (command === "/usr/bin/codesign" && args.includes("--force")) {
            signatures.push(args);
            fs.appendFileSync(executable, "-signed");
          }
          return { status: 0 };
        },
      },
    );
  }
  return { run, executable, signatures, swiftEnvironments, builds: () => builds };
}

it("reuses unchanged signed output without invoking Swift again", () => {
  const build = sandbox();
  build.run();
  build.run();
  expect(build.builds()).toBe(1);
  expect(fs.readFileSync(build.executable, "utf8")).toBe("trusted-build-signed");
});

it("drops the cargo placeholder from built and reused helpers", () => {
  const build = sandbox();
  const placeholder = path.join(
    path.dirname(path.dirname(build.executable)),
    "Resources/PLACEHOLDER.md",
  );
  fs.mkdirSync(path.dirname(placeholder), { recursive: true });
  fs.writeFileSync(placeholder, "No local transcriber in this build.\n");
  build.run();
  expect(fs.existsSync(placeholder)).toBe(false);
  fs.writeFileSync(placeholder, "No local transcriber in this build.\n");
  build.run();
  expect(build.builds()).toBe(1);
  expect(fs.existsSync(placeholder)).toBe(false);
});

it("rebuilds modified executable bytes instead of blessing them with a new signature", () => {
  const build = sandbox();
  build.run();
  fs.writeFileSync(build.executable, "altered-build-signed");
  build.run();
  expect(build.builds()).toBe(2);
  expect(fs.readFileSync(build.executable, "utf8")).toBe("trusted-build-signed");
  build.run();
  expect(build.builds()).toBe(2);
});

it("timestamps and hardens Developer ID release signatures, including reused output", () => {
  const build = sandbox({ release: true });
  build.run();
  build.run();
  expect(build.builds()).toBe(1);
  expect(build.signatures).toHaveLength(2);
  for (const args of build.signatures) {
    expect(args).toEqual(expect.arrayContaining(["--options", "runtime", "--timestamp"]));
    expect(args).not.toContain("--timestamp=none");
  }
});

it("uses installed Xcode for both the build and Swift output lookup", () => {
  const build = sandbox({ release: true });
  build.run();
  expect(build.swiftEnvironments).toHaveLength(2);
  for (const env of build.swiftEnvironments) {
    expect(env).toMatchObject({ DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer" });
  }
});
