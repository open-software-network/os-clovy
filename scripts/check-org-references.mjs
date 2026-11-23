#!/usr/bin/env node
// Fails when tracked files still reference the Open Software organization's
// infrastructure (its domains, the short-link domain, the GitHub organization,
// or Blacksmith runners). Each scope pairs git pathspecs with the pattern that
// must not match there; add a scope instead of widening an existing one.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const rootDir = path.resolve(path.dirname(scriptPath), "..");
const selfPath = path.relative(rootDir, scriptPath).split(path.sep).join("/");

export const ORG_REFERENCE_SCOPES = [
  {
    name: "repository and CI",
    pathspecs: [".github", "scripts", "Makefile"],
    pattern: /opensoftware\.co\b|june\.link|open-software-network|blacksmith/,
  },
  {
    // Tests are skipped: some pin repository and release metadata.
    name: "app source",
    pathspecs: ["src", "src-tauri/src", "src-tauri/*.json", ":!src/test"],
    pattern: /opensoftware\.co\b|june\.link|open-software-network/,
  },
  {
    // The June-era app identity (ADR-0068). Migration and native-secret
    // retirement may name legacy Keychain services to read or remove them.
    // Both modules remain covered by the service-domain scope above.
    name: "app identity",
    pathspecs: [
      "src",
      "src-tauri/src",
      "src-tauri/native",
      "src-tauri/build.rs",
      "src-tauri/Cargo.toml",
      "src-tauri/Info.plist",
      "src-tauri/*.json",
      "index.html",
      "onboarding-preview.html",
      "styleguide.html",
      ":!src/test",
      ":!src-tauri/src/identity_migration",
      ":!src-tauri/src/agent_sessions/retirement.rs",
    ],
    pattern: /co\.opensoftware|os-june|June\.app/,
  },
];

export function findOrgReferences({ cwd = rootDir, scopes = ORG_REFERENCE_SCOPES } = {}) {
  const findings = [];
  for (const scope of scopes) {
    const files = execFileSync("git", ["ls-files", "-z", "--", ...scope.pathspecs], {
      cwd,
      encoding: "utf8",
    })
      .split("\0")
      .filter((file) => file && file !== selfPath);
    for (const file of files) {
      let text;
      try {
        text = readFileSync(path.join(cwd, file), "utf8");
      } catch {
        continue;
      }
      text.split("\n").forEach((line, index) => {
        if (scope.pattern.test(line)) {
          findings.push({ scope: scope.name, file, line: index + 1, text: line.trim() });
        }
      });
    }
  }
  return findings;
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  const findings = findOrgReferences();
  for (const finding of findings) {
    console.error(`${finding.file}:${finding.line}: ${finding.text}`);
  }
  if (findings.length > 0) {
    console.error(`${findings.length} organization reference(s) found.`);
    process.exit(1);
  }
  console.log("No organization references found.");
}
