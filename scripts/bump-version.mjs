import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import process from "node:process";

const VERSION_RE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

export function parseSemver(version) {
  const match = VERSION_RE.exec(version);
  if (!match) return undefined;
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
  };
}

export function compareSemver(left, right) {
  const a = parseSemver(left);
  const b = parseSemver(right);
  if (!a || !b) throw new Error("Cannot compare invalid semver values.");
  for (const key of ["major", "minor", "patch"]) {
    if (a[key] > b[key]) return 1;
    if (a[key] < b[key]) return -1;
  }
  return 0;
}

export function validateRequestedVersion(currentVersion, requestedVersion) {
  if (!parseSemver(requestedVersion)) {
    return {
      ok: false,
      reason: `Requested version "${requestedVersion}" is not valid semver (expected X.Y.Z).`,
    };
  }
  if (!parseSemver(currentVersion)) {
    return {
      ok: false,
      reason: `Current version "${currentVersion}" is not valid semver.`,
    };
  }
  if (compareSemver(requestedVersion, currentVersion) <= 0) {
    return {
      ok: false,
      reason: `Requested version ${requestedVersion} must be greater than current version ${currentVersion}.`,
    };
  }
  return { ok: true };
}

export function bumpVersionContents(files, requestedVersion) {
  return {
    tauriConf: replaceJsonVersion(files.tauriConf, requestedVersion),
    cargoToml: replaceCargoPackageVersion(files.cargoToml, requestedVersion),
    cargoLock: replaceCargoLockPackageVersion(files.cargoLock, requestedVersion),
    packageJson: replaceJsonVersion(files.packageJson, requestedVersion),
  };
}

export function currentVersionFromTauriConf(contents) {
  return JSON.parse(contents).version;
}

export function currentVersionFromPackageJson(contents) {
  return JSON.parse(contents).version;
}

// Index of the `version = "..."` line inside the [package] table specifically,
// so a [workspace]/[dependencies] table's own version is never matched. The bare
// /^version/m first-match was fragile to table ordering.
function packageVersionLineIndex(lines) {
  let inPackage = false;
  for (let i = 0; i < lines.length; i++) {
    const trimmed = lines[i].trim();
    if (trimmed.startsWith("[")) {
      inPackage = trimmed === "[package]";
      continue;
    }
    if (inPackage && /^\s*version\s*=\s*"[^"]*"/.test(lines[i])) return i;
  }
  return -1;
}

export function currentVersionFromCargoToml(contents) {
  const lines = contents.split("\n");
  const index = packageVersionLineIndex(lines);
  if (index === -1) {
    throw new Error("Could not find [package] version in Cargo.toml.");
  }
  return /"([^"]+)"/.exec(lines[index])[1];
}

function cargoLockPackageTables(contents) {
  const headerPattern = /^[ \t]*(?:\[\[[^\]\r\n]+\]\]|\[[^\]\r\n]+\])[ \t]*(?:#.*)?\r?$/gm;
  const headers = [...contents.matchAll(headerPattern)];
  return headers
    .map((header, index) => ({
      start: header.index,
      end: headers[index + 1]?.index ?? contents.length,
      isPackage: /^[ \t]*\[\[package\]\]/.test(header[0]),
    }))
    .filter((table) => table.isPackage);
}

function cargoLockField(contents, table, field) {
  const tableContents = contents.slice(table.start, table.end);
  const fieldPattern = new RegExp(`^[ \\t]*${field}[ \\t]*=.*$`, "gm");
  const matches = [...tableContents.matchAll(fieldPattern)];
  if (matches.length !== 1) return { count: matches.length };

  const line = matches[0][0];
  const valueMatch = new RegExp(
    `^[ \\t]*${field}[ \\t]*=[ \\t]*"([^"\\r\\n]*)"[ \\t]*(?:#.*)?\\r?$`,
  ).exec(line);
  if (!valueMatch) return { count: 1, malformed: true };

  const valueStart =
    table.start + matches[0].index + line.indexOf(valueMatch[1], line.indexOf("="));
  return {
    count: 1,
    value: valueMatch[1],
    valueStart,
    valueEnd: valueStart + valueMatch[1].length,
  };
}

function desktopCargoLockPackage(contents) {
  const packageName = "clovy";
  const selected = [];
  for (const table of cargoLockPackageTables(contents)) {
    const name = cargoLockField(contents, table, "name");
    if (name.count !== 1 || name.malformed) continue;
    if (name.value === packageName) selected.push(table);
  }

  if (selected.length === 0) {
    throw new Error('Could not find a [[package]] named "clovy" in Cargo.lock.');
  }
  if (selected.length > 1) {
    throw new Error(`Found multiple [[package]] tables named "${packageName}" in Cargo.lock.`);
  }

  const table = selected[0];
  const version = cargoLockField(contents, table, "version");
  if (version.count !== 1) {
    throw new Error(
      `The Cargo.lock [[package]] named "${packageName}" must contain exactly one version field (found ${version.count}).`,
    );
  }
  if (version.malformed) {
    throw new Error(
      `The Cargo.lock [[package]] named "${packageName}" has a malformed version field.`,
    );
  }
  for (const field of ["source", "checksum"]) {
    if (cargoLockField(contents, table, field).count > 0) {
      throw new Error(
        `The Cargo.lock [[package]] named "${packageName}" unexpectedly contains a ${field} field.`,
      );
    }
  }
  return version;
}

export function currentVersionFromCargoLock(contents) {
  return desktopCargoLockPackage(contents).value;
}

// The four version-bearing files must already agree before a bump — otherwise
// the "strictly greater than current" gate would trust whichever file we read
// and silently carry a pre-existing drift into the release.
export function readCurrentVersion(files) {
  const tauri = currentVersionFromTauriConf(files.tauriConf);
  const cargo = currentVersionFromCargoToml(files.cargoToml);
  const lock = currentVersionFromCargoLock(files.cargoLock);
  const pkg = currentVersionFromPackageJson(files.packageJson);
  if (tauri !== cargo || tauri !== lock || tauri !== pkg) {
    return {
      ok: false,
      reason: `Version drift before bump (package.json=${pkg}, tauri.conf.json=${tauri}, Cargo.toml=${cargo}, Cargo.lock=${lock}); reconcile the four files first.`,
    };
  }
  return { ok: true, version: tauri };
}

function replaceJsonVersion(contents, requestedVersion) {
  const parsed = JSON.parse(contents);
  parsed.version = requestedVersion;
  return `${JSON.stringify(parsed, null, 2)}\n`;
}

function replaceCargoPackageVersion(contents, requestedVersion) {
  const lines = contents.split("\n");
  const index = packageVersionLineIndex(lines);
  if (index === -1) {
    throw new Error("Could not find [package] version in Cargo.toml.");
  }
  lines[index] = lines[index].replace(/version\s*=\s*"[^"]*"/, `version = "${requestedVersion}"`);
  return lines.join("\n");
}

function replaceCargoLockPackageVersion(contents, requestedVersion) {
  const version = desktopCargoLockPackage(contents);
  return `${contents.slice(0, version.valueStart)}${requestedVersion}${contents.slice(version.valueEnd)}`;
}

async function main() {
  const requestedVersion = process.argv[2];
  if (!requestedVersion) {
    throw new Error("Usage: node scripts/bump-version.mjs <version>");
  }

  const root = process.cwd();
  const paths = {
    tauriConf: resolve(root, "src-tauri/tauri.conf.json"),
    cargoToml: resolve(root, "src-tauri/Cargo.toml"),
    cargoLock: resolve(root, "src-tauri/Cargo.lock"),
    packageJson: resolve(root, "package.json"),
  };
  const files = {
    tauriConf: await readFile(paths.tauriConf, "utf8"),
    cargoToml: await readFile(paths.cargoToml, "utf8"),
    cargoLock: await readFile(paths.cargoLock, "utf8"),
    packageJson: await readFile(paths.packageJson, "utf8"),
  };
  const current = readCurrentVersion(files);
  if (!current.ok) {
    throw new Error(current.reason);
  }
  const validation = validateRequestedVersion(current.version, requestedVersion);
  if (!validation.ok) {
    throw new Error(validation.reason);
  }
  const next = bumpVersionContents(files, requestedVersion);
  await writeFile(paths.tauriConf, next.tauriConf);
  await writeFile(paths.cargoToml, next.cargoToml);
  await writeFile(paths.cargoLock, next.cargoLock);
  await writeFile(paths.packageJson, next.packageJson);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
