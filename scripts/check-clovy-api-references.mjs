#!/usr/bin/env node
// Inspect app/source/config/script files from both the Git index and worktree.
// Only ADRs and explicitly named historical documents are exempt; tests are covered except
// for this checker's one fixture suite, which deliberately plants violations.
import { execFileSync } from "node:child_process";
import { lstatSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, URL as NodeURL } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const defaultRoot = path.resolve(path.dirname(scriptPath), "..");
const exemptions = new Set([
  "scripts/check-clovy-api-references.mjs",
  "src/test/clovy-api-references-guard.test.mjs",
  // Retained history, not current setup instructions (ADR-0073).
  "docs/clovy-api-prd.md",
  "docs/telemetry-p3a-implementation-plan.md",
  "docs/superpowers/plans/2026-07-15-faster-note-transcription.md",
  "docs/superpowers/specs/2026-07-15-faster-note-transcription-design.md",
]);
const sourceDirectories = new Set([
  "docs",
  "src",
  "src-tauri",
  "scripts",
  ".github",
  "extension",
  "clovy-api",
  "agent-runtime",
  "crates",
]);
const rootConfig =
  /(?:\.(?:[cm]?[jt]sx?|md|rs|py|sh|ps1|jsonc?|ya?ml|toml|lock|html|css|plist|ini|conf|config)$|^(?:Makefile|Dockerfile|Justfile|\.env(?:\..*)?|\.npmrc|\.gitignore|\.gitattributes)$)/i;
const binaryExtension =
  /\.(?:png|jpe?g|gif|webp|avif|ico|icns|pdf|woff2?|ttf|otf|mp[34]|m4a|wav|aiff|flac|ogg|zip|gz|xz|bz2|dmg|exe|dll|dylib|so|a|o|sqlite3?|db)$/i;
const retiredRoot =
  /^\/v1\/(?:notes|dictate|models|chat\/completions|image|video|web|shares)(?=$|[/?#])/;
const bareRoute =
  /(?:^|[^A-Za-z0-9_/])(\/v1\/(?:notes|dictate|models|chat\/completions|image|video|web|shares))(?=$|[/?#"'`\s),;\]}]|\$\{|\{)/g;

function decodedPath(pathname) {
  try {
    return decodeURIComponent(pathname);
  } catch {
    return pathname;
  }
}

function retiredUrlRoute(candidate) {
  let url;
  try {
    url = new NodeURL(candidate);
  } catch {
    // A source-level host interpolation may not be a valid URL until runtime.
    const pathname = candidate.replace(/^https?:\/\/[^/?#]*/i, "").split(/[?#]/, 1)[0];
    return decodedPath(pathname).match(retiredRoot)?.[0];
  }
  if (
    url.protocol === "https:" &&
    url.hostname === "api.fal.ai" &&
    !url.port &&
    !url.username &&
    !url.password &&
    url.pathname === "/v1/models/pricing" &&
    !url.search &&
    !url.hash
  )
    return undefined;
  return decodedPath(url.pathname).match(retiredRoot)?.[0];
}

export function scanClovyApiText(text) {
  const findings = [];
  for (const [index, line] of text.split("\n").entries()) {
    const add = (match) => findings.push({ line: index + 1, match });
    // Identifiers are forbidden substrings, including inside URLs or longer names.
    for (const match of line.matchAll(
      /CLOVY_API_URL|clovy_api_url|OS_CLOVY_LOCAL_DEV_BEARER_TOKEN|local_dev_bearer_token/g,
    ))
      add(match[0]);
    // Match complete URL spans before bare routes, so provider prefixes and
    // query examples are interpreted as URLs instead of arbitrary substrings.
    const withoutUrls = line.replace(/https?:\/\/[^\s<>"'`]+/gi, (span, offset) => {
      const quoted = /["'`]/.test(line[offset - 1] ?? "");
      const candidate = quoted ? span : span.replace(/[),;\]}]+$/, "");
      const route = retiredUrlRoute(candidate);
      if (route) add(route);
      return " ".repeat(span.length);
    });
    for (const match of withoutUrls.matchAll(bareRoute)) add(match[1]);
  }
  return findings;
}

function inScope(file) {
  if (exemptions.has(file) || file.startsWith("docs/adr/") || binaryExtension.test(file))
    return false;
  const parts = file.split("/");
  return parts.length > 1 ? sourceDirectories.has(parts[0]) : rootConfig.test(file);
}

export function findClovyApiReferences({ root = defaultRoot } = {}) {
  const files = execFileSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    },
  ).split("\0");
  const findings = [];
  const seen = new Set();
  const scan = (file, bytes) => {
    if (bytes.includes(0)) return;
    for (const finding of scanClovyApiText(bytes.toString("utf8"))) {
      const key = JSON.stringify([file, finding.line, finding.match]);
      if (seen.has(key)) continue;
      seen.add(key);
      findings.push({ file, ...finding });
    }
  };
  // Read index blobs separately: a cleaned/deleted worktree file must not hide
  // an unsafe staged version. Object ids avoid interpreting file names as revisions.
  const entries = execFileSync("git", ["ls-files", "--stage", "-z"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  })
    .split("\0")
    .filter(Boolean)
    .map((entry) => {
      const tab = entry.indexOf("\t");
      const [mode, oid] = entry.slice(0, tab).split(" ");
      return { mode, oid, file: entry.slice(tab + 1) };
    })
    .filter(({ mode, file }) => mode.startsWith("100") && inScope(file));
  for (let start = 0; start < entries.length; start += 128) {
    const batch = entries.slice(start, start + 128);
    const output = execFileSync("git", ["cat-file", "--batch"], {
      cwd: root,
      input: `${batch.map(({ oid }) => oid).join("\n")}\n`,
      maxBuffer: 64 * 1024 * 1024,
    });
    let offset = 0;
    for (const { file } of batch) {
      const end = output.indexOf(10, offset);
      const [, kind, sizeText] = output.subarray(offset, end).toString("utf8").split(" ");
      const size = Number(sizeText);
      if (
        end < 0 ||
        kind !== "blob" ||
        !Number.isSafeInteger(size) ||
        size < 0 ||
        end + size + 2 > output.length
      ) {
        throw new Error("Could not read an indexed file for the reference guard.");
      }
      scan(file, output.subarray(end + 1, end + 1 + size));
      offset = end + size + 2;
    }
  }
  for (const file of [...new Set(files)].filter(inScope).sort()) {
    let bytes;
    try {
      const absolute = path.join(root, file);
      if (!lstatSync(absolute).isFile()) continue;
      bytes = readFileSync(absolute);
    } catch (error) {
      if (error.code === "ENOENT") continue;
      throw error;
    }
    scan(file, bytes);
  }
  return findings;
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  const args = process.argv.slice(2);
  if (args.length !== 0 && (args.length !== 2 || args[0] !== "--root")) {
    console.error("Usage: check-clovy-api-references.mjs [--root checkout]");
    process.exitCode = 2;
  } else {
    const findings = findClovyApiReferences({
      root: args.length ? path.resolve(args[1]) : defaultRoot,
    });
    for (const finding of findings) {
      // Report only the offending root/name, never a URL's credentials or query.
      console.error(`${finding.file}:${finding.line}: ${finding.match}`);
    }
    if (findings.length > 0) {
      console.error(`${findings.length} retired API reference(s) found.`);
      process.exitCode = 1;
    } else {
      console.log("No retired API references found.");
    }
  }
}
