#!/usr/bin/env node
// Metadata only for LFS weights. Small Git blobs are hashed in memory, never installed.
import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repository = "FluidInference/parakeet-ultra-coreml";
const revision = "95eaa59a39d4394f047a4dc5cce480388a60d1b6";
const prefix = `https://huggingface.co/${repository}/resolve/${revision}`;
const required =
  /^(?:(?:Preprocessor|Encoder|Decoder|JointDecisionv3)\.mlmodelc\/|parakeet_vocab\.json$)/;
let url = `https://huggingface.co/api/models/${repository}/tree/${revision}?recursive=true&expand=true`;
const entries = [];
while (url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Tree API: ${response.status}`);
  entries.push(...(await response.json()));
  url = response.headers.get("link")?.match(/<([^>]+)>; rel="next"/)?.[1];
}
const files = [];
for (const entry of entries
  .filter((item) => item.type === "file" && required.test(item.path))
  .sort((a, b) => a.path.localeCompare(b.path, "en"))) {
  let sha256 = entry.lfs?.oid;
  if (!sha256) {
    const response = await fetch(`${prefix}/${entry.path}`);
    if (!response.ok) throw new Error(`${entry.path}: ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length !== entry.size) throw new Error(`${entry.path}: size differs`);
    const gitHash = createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
    if (gitHash !== entry.oid) throw new Error(`${entry.path}: Git object differs`);
    sha256 = createHash("sha256").update(bytes).digest("hex");
  }
  if (!/^[a-f0-9]{64}$/.test(sha256)) throw new Error(`${entry.path}: invalid SHA-256`);
  files.push({ path: entry.path, bytes: entry.size, sha256 });
}
for (const component of [
  "Preprocessor.mlmodelc/",
  "Encoder.mlmodelc/",
  "Decoder.mlmodelc/",
  "JointDecisionv3.mlmodelc/",
  "parakeet_vocab.json",
]) {
  if (!files.some((file) => file.path.startsWith(component)))
    throw new Error(`Missing ${component}`);
}
const manifest = {
  repository,
  revision,
  license: "CC-BY-4.0",
  total_bytes: files.reduce((sum, file) => sum + file.bytes, 0),
  files,
};
writeFileSync(
  path.join(root, "src-tauri/resources/local-transcriber-model.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
console.log(`${files.length} files; ${manifest.total_bytes} bytes; revision ${revision}`);
