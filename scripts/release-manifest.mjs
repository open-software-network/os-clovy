// Writes the updater manifest (latest.json) for a release from the build
// artifacts: one universal macOS package for both Mac architectures and the
// Windows NSIS installer when selected, each with its minisign .sig contents.
//   node scripts/release-manifest.mjs --tag v0.1.0 --dir <artifacts> --out latest.json
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const MAC_PACKAGE = "Clovy.app.tar.gz";
const MAC_DMG = "Clovy_universal.dmg";
const WINDOWS_INSTALLER_RE = /_x64-setup\.exe$/;

function required(dir, name) {
  const names = readdirSync(dir);
  if (!names.includes(name)) throw new Error(`Missing release artifact ${name} in ${dir}.`);
  return join(dir, name);
}

function validatePlatforms(platforms) {
  if (platforms !== "all" && platforms !== "macos") {
    throw new Error(
      `Invalid release platforms ${JSON.stringify(platforms)}: expected all or macos.`,
    );
  }
}

export function readReleaseAssets(dir, platforms = "all") {
  validatePlatforms(platforms);
  const macPackage = required(dir, MAC_PACKAGE);
  const macSignature = required(dir, `${MAC_PACKAGE}.sig`);
  const dmg = required(dir, MAC_DMG);
  const signature = (path) => readFileSync(path, "utf8").trim();
  const assets = {
    mac: { name: MAC_PACKAGE, signature: signature(macSignature) },
    files: [dmg, macPackage, macSignature],
  };
  if (platforms === "macos") return assets;
  const installers = readdirSync(dir).filter((name) => WINDOWS_INSTALLER_RE.test(name));
  if (installers.length !== 1) {
    throw new Error(`Expected one Windows *_x64-setup.exe in ${dir}, found ${installers.length}.`);
  }
  const windowsInstaller = join(dir, installers[0]);
  const windowsSignature = required(dir, `${installers[0]}.sig`);
  assets.windows = { name: installers[0], signature: signature(windowsSignature) };
  assets.files.push(windowsInstaller, windowsSignature);
  return assets;
}

export function buildReleaseManifest({ tag, repository, pubDate, assets, platforms = "all" }) {
  validatePlatforms(platforms);
  const base = `https://github.com/${repository}/releases/download/${tag}`;
  const entry = (asset) => ({
    signature: asset.signature,
    url: `${base}/${encodeURIComponent(asset.name)}`,
  });
  const mac = entry(assets.mac);
  const entries = { "darwin-aarch64": mac, "darwin-x86_64": mac };
  if (platforms === "all") entries["windows-x86_64"] = entry(assets.windows);
  return {
    version: tag.replace(/^v/, ""),
    pub_date: pubDate,
    platforms: entries,
  };
}

function option(args, name) {
  const index = args.indexOf(name);
  if (index === -1) return undefined;
  const value = args[index + 1];
  if (value === undefined || value.startsWith("--")) throw new Error(`Missing value for ${name}.`);
  return value;
}

function main() {
  const args = process.argv.slice(2);
  const platforms = option(args, "--platforms") ?? "all";
  validatePlatforms(platforms);
  const tag = option(args, "--tag");
  const dir = option(args, "--dir");
  const out = option(args, "--out") ?? "latest.json";
  const repository = option(args, "--repo") ?? process.env.GITHUB_REPOSITORY;
  if (!tag || !dir || !repository) {
    throw new Error(
      "Usage: node scripts/release-manifest.mjs --tag vX.Y.Z --dir <artifacts> [--out latest.json] [--repo owner/name] [--platforms all|macos]",
    );
  }
  const assets = readReleaseAssets(dir, platforms);
  const manifest = buildReleaseManifest({
    tag,
    repository,
    pubDate: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
    assets,
    platforms,
  });
  writeFileSync(out, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Wrote ${out} for ${tag}: ${Object.keys(manifest.platforms).join(", ")}`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
