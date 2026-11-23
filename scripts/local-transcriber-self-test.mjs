#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// No default owner profile: the caller supplies the run-owned app data directory.
const dataDir = process.argv[2];
const modelDir = dataDir && path.join(path.resolve(dataDir), "models/parakeet-ultra-coreml");
if (!modelDir || !existsSync(path.join(modelDir, "installed.json"))) {
  console.error(
    "Skipped: pass an isolated dev app data directory with the model installed through Settings.",
  );
  process.exit(0);
}
const executable = path.join(
  root,
  ".tauri-helper/Clovy Local Transcriber.app/Contents/MacOS/clovy-local-transcriber",
);
const requests = [
  { op: "load", model_dir: modelDir, version: "ultra" },
  {
    op: "transcribe",
    audio_path: path.join(root, "src-tauri/tests/fixtures/parakeet-pt.wav"),
    language: "pt",
  },
  { op: "unload" },
];
const result = spawnSync(executable, [], {
  input: `${requests.map((request) => JSON.stringify(request)).join("\n")}\n`,
  encoding: "utf8",
  timeout: 180_000,
  maxBuffer: 4 * 1024 * 1024,
});
assert.equal(result.status, 0, result.stderr);
const responses = result.stdout
  .trim()
  .split("\n")
  .map((line) => JSON.parse(line));
assert.equal(responses.length, 3);
for (const response of responses) assert.equal(response.ok, true, response.error);
assert.match(responses[1].text.toLocaleLowerCase("pt-BR"), /reunião/);
assert.match(responses[1].text.toLocaleLowerCase("pt-BR"), /português/);
assert.ok(responses[1].tokens.length > 0);
console.log(JSON.stringify(responses, null, 2));
