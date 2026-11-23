import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  findClovyApiReferences,
  scanClovyApiText,
} from "../../scripts/check-clovy-api-references.mjs";

const script = path.resolve("scripts/check-clovy-api-references.mjs");
const roots = [];
function checkout() {
  const root = mkdtempSync(path.join(os.tmpdir(), "clovy-reference-guard-"));
  roots.push(root);
  execFileSync("git", ["init", "--quiet", root]);
  return root;
}
function put(root, name, text, tracked = false) {
  const destination = path.join(root, name);
  mkdirSync(path.dirname(destination), { recursive: true });
  writeFileSync(destination, text);
  if (tracked) execFileSync("git", ["-C", root, "add", "--", name]);
}
function run(root) {
  return spawnSync(process.execPath, [script, "--root", root], { encoding: "utf8" });
}
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("retired API reference guard", () => {
  it("flags both identifiers even inside longer identifiers", () => {
    for (const sample of [
      "CLOVY_API_URL",
      "clovy_api_url",
      "PREFIX_CLOVY_API_URL_SUFFIX",
      "old_clovy_api_url_resolver",
    ]) {
      expect(scanClovyApiText(sample), sample).toHaveLength(1);
    }
  });

  it("rejects local bearer names inside source, longer identifiers and URLs", () => {
    for (const name of ["OS_CLOVY_LOCAL_DEV_BEARER_TOKEN", "local_dev_bearer_token"]) {
      for (const sample of [name, `prefix_${name}_suffix`, `https://example.test/?name=${name}`]) {
        expect(scanClovyApiText(sample), sample).toEqual([{ line: 1, match: name }]);
      }
    }
    const root = checkout();
    put(root, "src/token.ts", "// fixture\nconst token = local_dev_bearer_token;\n");
    put(root, ".env.example", "OS_CLOVY_LOCAL_DEV_BEARER_TOKEN=fixture");
    const rejected = run(root);
    expect(rejected.status).toBe(1);
    expect(rejected.stderr).toContain("src/token.ts:2: local_dev_bearer_token");
    expect(rejected.stderr).toContain(".env.example:1: OS_CLOVY_LOCAL_DEV_BEARER_TOKEN");
  });

  it("flags every retired root, descendant, query and interpolated path", () => {
    for (const root of [
      "notes",
      "dictate",
      "models",
      "chat/completions",
      "image",
      "video",
      "web",
      "shares",
    ]) {
      for (const sample of [
        `"/v1/${root}"`,
        `"/v1/${root}/child"`,
        `"/v1/${root}?x=1"`,
        `format!("{base}/v1/${root}")`,
        `"/v1/${root}{query}"`,
        `https://localhost:8080/v1/${root}`,
      ])
        expect(scanClovyApiText(sample), sample).toHaveLength(1);
      expect(scanClovyApiText(`"/v1/${root}_other"`)).toEqual([]);
    }
  });

  it("parses full URLs before checking their paths", () => {
    for (const sample of [
      "https://openrouter.ai/api/v1/models",
      "https://openrouter.ai/api/v1/chat/completions",
      "https://api.groq.com/openai/v1/models",
      "https://api.groq.com/openai/v1/audio/transcriptions",
      '"/api/v1/models"',
      '"/openai/v1/models"',
      "https://example.test/help?example=/v1/models",
    ])
      expect(scanClovyApiText(sample), sample).toEqual([]);
    expect(scanClovyApiText("https://example.test/api/../v1/models")).toHaveLength(1);
    expect(scanClovyApiText("https://example.test/%76%31/models")).toHaveLength(1);
  });

  it("allows only the exact fal pricing endpoint without credentials or extra components", () => {
    expect(scanClovyApiText("https://api.fal.ai/v1/models/pricing")).toEqual([]);
    expect(scanClovyApiText("https://api.fal.ai:443/v1/models/pricing")).toEqual([]);
    for (const sample of [
      "http://api.fal.ai/v1/models/pricing",
      "https://api.fal.ai:444/v1/models/pricing",
      "https://user:secret@api.fal.ai/v1/models/pricing",
      "https://api.fal.ai@evil.test/v1/models/pricing",
      "https://api.fal.ai.evil.test/v1/models/pricing",
      "https://evil.test/v1/models/pricing",
      "https://api.fal.ai/v1/models/pricing/extra",
      "https://api.fal.ai/v1/models/pricing/",
      "https://api.fal.ai/v1/models/pricing?extra=1",
      "https://api.fal.ai/v1/models/pricing#extra",
      "https://api.fal.ai/v1/models/%70ricing",
      "https://api.fal.ai/v1/models/pricing/../other",
    ])
      expect(scanClovyApiText(sample), sample).toHaveLength(1);
  });

  it("masks only each URL span, never the whole line or identifier matches", () => {
    const text = `fetch("https://api.fal.ai/v1/models/pricing"); fetch(\`\${base}/v1/models\`); CLOVY_API_URL`;
    expect(scanClovyApiText(text)).toEqual([
      { line: 1, match: "CLOVY_API_URL" },
      { line: 1, match: "/v1/models" },
    ]);
    expect(scanClovyApiText("https://example.test/?key=clovy_api_url")).toEqual([
      { line: 1, match: "clovy_api_url" },
    ]);
  });

  it("does not trim suffix characters from a quoted fal URL", () => {
    for (const suffix of ["}", ")", "]", ";", ","]) {
      expect(scanClovyApiText(`"https://api.fal.ai/v1/models/pricing${suffix}"`)).toHaveLength(1);
    }
  });

  it("scans source, configuration, scripts and tests with only exact self exemptions", () => {
    const root = checkout();
    for (const name of [
      "src/test/other.test.mjs",
      "src-tauri/tests/guard.rs",
      "scripts/helper.mjs",
      "package.json",
      "Makefile",
      ".env.example",
      ".github/workflows/build.yml",
      "docs/current.md",
    ]) {
      put(root, name, "CLOVY_API_URL");
    }
    for (const name of [
      "scripts/check-clovy-api-references.mjs",
      "src/test/clovy-api-references-guard.test.mjs",
      "docs/adr/0001-history.md",
      "openspec/changes/history/spec.md",
    ]) {
      put(root, name, "CLOVY_API_URL");
    }
    expect(findClovyApiReferences({ root })).toHaveLength(8);
  });

  it("scans current documentation while preserving named historical records", () => {
    const root = checkout();
    for (const name of ["README.md", "docs/development.md", "docs/new-guide.md"]) {
      put(root, name, "CLOVY_API_URL");
    }
    for (const name of ["docs/adr/0001-history.md", "docs/clovy-api-prd.md"]) {
      put(root, name, "CLOVY_API_URL");
    }
    expect(findClovyApiReferences({ root }).map(({ file }) => file)).toEqual([
      "README.md",
      "docs/development.md",
      "docs/new-guide.md",
    ]);
  });

  it("rejects a staged violation even when the worktree was cleaned or deleted", () => {
    const root = checkout();
    put(root, "src/staged.ts", "CLOVY_API_URL", true);
    put(root, "src/staged.ts", "export const direct = true;");
    expect(run(root).status).toBe(1);
    rmSync(path.join(root, "src/staged.ts"));
    expect(run(root).status).toBe(1);
    execFileSync("git", ["-C", root, "add", "-u"]);
    expect(run(root).status).toBe(0);
  });

  it("CLI rejects tracked and untracked references with file:line, then passes after removal", () => {
    const root = checkout();
    put(root, "src/tracked.ts", "// first line\nconst url = CLOVY_API_URL;\n", true);
    put(root, "src/untracked.ts", '// first line\nfetch("/v1/notes/transcribe");\n');
    const rejected = run(root);
    expect(rejected.status).toBe(1);
    expect(rejected.stderr).toContain("src/tracked.ts:2: CLOVY_API_URL");
    expect(rejected.stderr).toContain("src/untracked.ts:2: /v1/notes");
    put(root, "src/tracked.ts", "export const local = true;\n", true);
    rmSync(path.join(root, "src/untracked.ts"));
    const clean = run(root);
    expect(clean.status).toBe(0);
    expect(clean.stdout).toContain("No retired API references found.");
  });

  it("reports both keys and route on their exact lines without printing URL credentials", () => {
    const root = checkout();
    put(
      root,
      "scripts/example.mjs",
      "CLOVY_API_URL\nclovy_api_url\nhttps://user:secret@api.fal.ai/v1/models/pricing\n",
    );
    const output = run(root);
    expect(output.status).toBe(1);
    expect(output.stderr).toContain("scripts/example.mjs:1: CLOVY_API_URL");
    expect(output.stderr).toContain("scripts/example.mjs:2: clovy_api_url");
    expect(output.stderr).toContain("scripts/example.mjs:3: /v1/models");
    expect(output.stderr).not.toContain("secret");
  });
});
