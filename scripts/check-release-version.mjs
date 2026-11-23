// Fails a release before any build when the tag does not match the version in
// the four version files, or when a release for the tag already exists. A dry
// run publishes nothing, so it may rehearse a version already released.
//   node scripts/check-release-version.mjs v0.1.0 [--repo owner/name] [--dry-run]
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";
import { readCurrentVersion } from "./bump-version.mjs";

const TAG_RE = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

export function checkReleaseVersion({ tag, files, releaseExists, dryRun = false }) {
  if (!TAG_RE.test(tag)) {
    return { ok: false, reason: `Tag "${tag}" is not vX.Y.Z.` };
  }
  const current = readCurrentVersion(files);
  if (!current.ok) return current;
  if (tag !== `v${current.version}`) {
    return {
      ok: false,
      reason: `Tag ${tag} does not match version ${current.version} in package.json, Cargo.toml, Cargo.lock and tauri.conf.json; run pnpm version:bump first.`,
    };
  }
  if (releaseExists && !dryRun) {
    return { ok: false, reason: `A release for ${tag} already exists.` };
  }
  return { ok: true, version: current.version };
}

function releaseExistsOnGitHub(tag, repository) {
  const args = ["release", "view", tag, "--json", "tagName"];
  if (repository) args.push("--repo", repository);
  const result = spawnSync("gh", args, { encoding: "utf8" });
  if (result.status === 0) return true;
  if (/release not found/i.test(result.stderr)) return false;
  throw new Error(`Could not ask GitHub whether ${tag} exists: ${result.stderr.trim()}`);
}

async function main() {
  const [tag, ...rest] = process.argv.slice(2);
  if (!tag)
    throw new Error("Usage: node scripts/check-release-version.mjs <vX.Y.Z> [--repo owner/name]");
  const repoIndex = rest.indexOf("--repo");
  const repository = repoIndex === -1 ? process.env.GITHUB_REPOSITORY : rest[repoIndex + 1];
  const dryRun = rest.includes("--dry-run");
  const root = process.cwd();
  const read = (path) => readFile(resolve(root, path), "utf8");
  const files = {
    tauriConf: await read("src-tauri/tauri.conf.json"),
    cargoToml: await read("src-tauri/Cargo.toml"),
    cargoLock: await read("src-tauri/Cargo.lock"),
    packageJson: await read("package.json"),
  };
  const result = checkReleaseVersion({
    tag,
    files,
    releaseExists: releaseExistsOnGitHub(tag, repository),
    dryRun,
  });
  if (!result.ok) throw new Error(result.reason);
  console.log(
    dryRun
      ? `Dry run: ${tag} matches version ${result.version}; nothing is published.`
      : `Release ${tag} matches version ${result.version} and is not published yet.`,
  );
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
